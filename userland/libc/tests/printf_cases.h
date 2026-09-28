/*
 * printf_cases.h - the printf conformance table, shared by TWO programs:
 *
 *   1. libc_host_test.c runs every CASE through NovaOS's own
 *      vsnprintf() (via do_case()).
 *   2. printf_oracle.c runs the identical table through the host C
 *      library's snprintf() and checks that each "expected" string is
 *      what a real C library actually produces.
 *
 * So a case here is only trusted once a real libc has agreed with it -
 * the expected strings are not merely what this author believes C does.
 * Anything NovaOS deliberately does differently from glibc (an
 * unsupported "%lld", say) lives in libc_host_test.c instead, not in
 * this shared table.
 *
 * CASE(expected, format, args...)
 */

/* literals and the basic conversions */
CASE("hello", "hello")
CASE("42", "%d", 42)
CASE("-42", "%d", -42)
CASE("0", "%d", 0)
CASE("2147483647", "%d", 2147483647)
CASE("-2147483648", "%d", (-2147483647 - 1))
CASE("4294967295", "%u", 4294967295u)
CASE("ff", "%x", 255)
CASE("FF", "%X", 255)
CASE("deadbeef", "%x", 0xdeadbeefu)
CASE("0", "%x", 0)
CASE("17", "%o", 15)
CASE("0", "%o", 0)
CASE("a", "%c", 'a')
CASE("abc", "%s", "abc")
CASE("", "%s", "")
CASE("100%", "100%%")
CASE("%", "%%")
CASE("i=7", "i=%i", 7)

/* alternate form */
CASE("0xff", "%#x", 255)
CASE("0XFF", "%#X", 255)
CASE("0", "%#x", 0)
CASE("017", "%#o", 15)

/* width and flags */
CASE("   42", "%5d", 42)
CASE("42   |", "%-5d|", 42)
CASE("00042", "%05d", 42)
CASE("-0042", "%05d", -42)
CASE("  abc", "%5s", "abc")
CASE("abc  |", "%-5s|", "abc")
CASE("abc", "%2s", "abc")
CASE("    x", "%5c", 'x')
CASE("x    |", "%-5c|", 'x')
CASE("+5", "%+d", 5)
CASE("-5", "%+d", -5)
CASE("+0", "%+d", 0)
CASE(" 5", "% d", 5)
CASE("   +5", "%+5d", 5)
CASE("+5   |", "%-+5d|", 5)
CASE("+0005", "%+05d", 5)
CASE(" 0005", "% 05d", 5)
CASE("5    |", "%0-5d|", 5)
CASE(" 0xff", "%#5x", 255)
CASE("0x0ff", "%#05x", 255)
CASE("   ff", "%5x", 255)
CASE("000ff", "%05x", 255)

/* precision */
CASE("abc", "%.3s", "abcdef")
CASE("ab", "%.2s", "abcdef")
CASE("", "%.0s", "abc")
CASE("  ab", "%4.2s", "abcdef")
CASE("007", "%.3d", 7)
CASE("  007", "%5.3d", 7)
CASE("-007", "%.3d", -7)
CASE("", "%.0d", 0)
CASE("1", "%.0d", 1)
CASE("00ff", "%.4x", 255)
CASE("  00ff", "%6.4x", 255)
CASE("0x00ff", "%#.4x", 255)
CASE("007  |", "%-5.3d|", 7)
CASE("  007", "%05.3d", 7)

/* '*' width and precision */
CASE("   42", "%*d", 5, 42)
CASE("42   |", "%-*d|", 5, 42)
CASE("42   |", "%*d|", -5, 42)
CASE("abc", "%.*s", 3, "abcdef")
CASE("  ab", "%*.*s", 4, 2, "abcdef")

/* length modifiers (all 32-bit on this target except h/hh, which cast) */
CASE("42", "%ld", 42L)
CASE("42", "%lu", 42UL)
CASE("ff", "%lx", 255UL)
CASE("42", "%zu", (size_t)42)
CASE("-1", "%hd", (short)-1)
CASE("65535", "%hu", (unsigned short)65535)
CASE("4464", "%hd", 70000)
CASE("112", "%hhd", 368)
CASE("-1", "%hhd", 255)
CASE("ff", "%hhx", 0x1ff)

/* pointers (fixed values, so glibc and NovaOS agree) */
CASE("0x1234", "%p", (void*)0x1234)
CASE("0xdeadbeef", "%p", (void*)0xdeadbeefu)

/* several conversions together */
CASE("a=1 b=two c=3", "a=%d b=%s c=%x", 1, "two", 3)
CASE("[  7][7  ][007]", "[%3d][%-3d][%03d]", 7, 7, 7)
CASE("x=-5 y=+5 z= 5", "x=%d y=%+d z=% d", -5, 5, 5)
CASE("id:0007 name:  bob", "id:%04d name:%5s", 7, "bob")
CASE("100% done", "%d%% done", 100)
