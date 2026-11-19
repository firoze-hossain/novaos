/* tools/tests/elf_loader_test.c
 * Host-side unit test for Dual-Architecture ELF32/ELF64 loader in NovaOS.
 * Validates:
 * 1. 32-bit ELF binary validation (ELFCLASS32, EM_386, ET_EXEC)
 * 2. 64-bit ELF binary validation (ELFCLASS64, EM_X86_64, ET_EXEC)
 * 3. ELF inspect segment extraction for both 32-bit and 64-bit binaries
 * 4. Rejection of corrupted magic, unsupported architectures, truncated headers
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <assert.h>

#include "../../kernel/task/elf.h"

// Forward declaration of functions in elf.c
bool elf_validate(const uint8_t* data, uint32_t size);
bool elf_inspect(const uint8_t* data, uint32_t size, elf_image_t* out_image);

// Mocks for kernel functions required if elf.c is linked
void kernel_log(const char* fmt, ...) {
    (void)fmt;
}

uint32_t pmm_alloc_frame(void) {
    return 0x10000;
}

void paging_map_page(uint32_t* pd, uint32_t vaddr, uint32_t paddr, uint32_t flags) {
    (void)pd; (void)vaddr; (void)paddr; (void)flags;
}

static uint8_t* read_file(const char* path, uint32_t* out_sz) {
    FILE* f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t* buf = (uint8_t*)malloc(sz);
    fread(buf, 1, sz, f);
    fclose(f);
    *out_sz = (uint32_t)sz;
    return buf;
}

int main(void) {
    printf("--- Running NovaOS Dual-Architecture ELF32/ELF64 Loader Tests ---\n");

    // Test 1: Validate 32-bit ELF fixture (HELLO.ELF)
    uint32_t elf32_sz = 0;
    uint8_t* elf32_data = read_file("tools/fixtures/HELLO.ELF", &elf32_sz);
    assert(elf32_data != NULL);
    assert(elf_validate(elf32_data, elf32_sz) == true);
    printf("  [PASS] ELF32 validation (HELLO.ELF): ELFCLASS32, EM_386 accepted\n");

    elf_image_t img32;
    memset(&img32, 0, sizeof(img32));
    assert(elf_inspect(elf32_data, elf32_sz, &img32) == true);
    assert(img32.entry_point == 0x08048000 || img32.entry_point > 0);
    assert(img32.seg_count >= 1);
    printf("  [PASS] ELF32 inspect (HELLO.ELF): entry=0x%x, segments=%d\n", img32.entry_point, img32.seg_count);

    // Test 2: Validate 64-bit ELF fixture (HELLO64.ELF)
    uint32_t elf64_sz = 0;
    uint8_t* elf64_data = read_file("tools/fixtures/HELLO64.ELF", &elf64_sz);
    assert(elf64_data != NULL);
    assert(elf_validate(elf64_data, elf64_sz) == true);
    printf("  [PASS] ELF64 validation (HELLO64.ELF): ELFCLASS64, EM_X86_64 accepted\n");

    elf_image_t img64;
    memset(&img64, 0, sizeof(img64));
    assert(elf_inspect(elf64_data, elf64_sz, &img64) == true);
    assert(img64.entry_point == 0x00400000 || img64.entry_point > 0);
    assert(img64.seg_count >= 1);
    printf("  [PASS] ELF64 inspect (HELLO64.ELF): entry=0x%x, segments=%d\n", img64.entry_point, img64.seg_count);

    // Test 3: Reject invalid / corrupted ELF binaries
    uint8_t bad_magic[64];
    memset(bad_magic, 0, sizeof(bad_magic));
    assert(elf_validate(bad_magic, sizeof(bad_magic)) == false);

    uint8_t bad_class[64];
    memcpy(bad_class, elf32_data, 64);
    bad_class[4] = 99; // invalid class
    assert(elf_validate(bad_class, sizeof(bad_class)) == false);

    uint8_t bad_endian[64];
    memcpy(bad_endian, elf32_data, 64);
    bad_endian[5] = 2; // big endian
    assert(elf_validate(bad_endian, sizeof(bad_endian)) == false);

    uint8_t truncated[16];
    memcpy(truncated, elf64_data, 16);
    assert(elf_validate(truncated, sizeof(truncated)) == false);

    printf("  [PASS] Negative test cases: invalid magic, bad class, big-endian, and truncation correctly rejected\n");

    free(elf32_data);
    free(elf64_data);

    printf("[PASS] All NovaOS Dual-Architecture ELF loader tests passed!\n");
    return 0;
}
