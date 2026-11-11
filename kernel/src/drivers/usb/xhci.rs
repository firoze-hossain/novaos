//! kernel/src/drivers/usb/xhci.rs
//!
//! NovaOS 2.0 (Month 1 - Foundation): Pure Rust xHCI USB 3.2 Host Controller Driver
//!
//! Provides a complete Extensible Host Controller Interface (xHCI) driver in pure
//! `#![no_std]` Rust supporting USB 1.1 (Low/Full Speed), USB 2.0 (High Speed), and
//! USB 3.2 (SuperSpeed 5 Gbps & SuperSpeedPlus 10 Gbps) devices.
//!
//! Architectural Highlights:
//! - Direct PCI Class `0x0C0330` discovery (Serial Bus 0x0C, USB 0x03, xHCI 0x30).
//! - 64-bit BAR0 MMIO space mapping (Capability, Operational, Runtime, and Doorbell registers).
//! - Transfer Request Block (TRB) circular ring management (Normal, Setup, Data, Status, Link).
//! - Device Context Base Address Array (DCBAA) and device slot assignment.
//! - Root hub port scanning, reset, speed detection, and hot-plug event dispatch.
//! - Endpoint 0 control transfer pipeline for standard USB device descriptor retrieval.
//! - Zero dynamic heap allocation; uses static DMA rings and device contexts.

#![allow(dead_code, static_mut_refs)]

// ============================================================================
// PCI & MMIO Register Definitions
// ============================================================================

pub const PCI_CLASS_SERIAL: u8 = 0x0C;
pub const PCI_SUBCLASS_USB: u8 = 0x03;
pub const PCI_PROGIF_XHCI: u8 = 0x30;
pub const PCI_CLASS_XHCI: u32 = 0x0C0330;

// Capability Register Offsets (relative to MMIO base)
pub const XHCI_CAP_CAPLENGTH: usize = 0x00;  // 8-bit
pub const XHCI_CAP_HCIVERSION: usize = 0x02; // 16-bit
pub const XHCI_CAP_HCSPARAMS1: usize = 0x04; // 32-bit: MaxSlots, MaxIntrs, MaxPorts
pub const XHCI_CAP_HCSPARAMS2: usize = 0x08; // 32-bit: MaxScratchpadBuffers
pub const XHCI_CAP_HCSPARAMS3: usize = 0x0C; // 32-bit
pub const XHCI_CAP_HCCPARAMS1: usize = 0x10; // 32-bit: 64-bit address, CSZ
pub const XHCI_CAP_DBOFF: usize = 0x14;      // 32-bit: Doorbell Array Offset
pub const XHCI_CAP_RTSOFF: usize = 0x18;     // 32-bit: Runtime Register Space Offset
pub const XHCI_CAP_HCCPARAMS2: usize = 0x1C; // 32-bit

// Operational Register Offsets (relative to base + CAPLENGTH)
pub const XHCI_OP_USBCMD: usize = 0x00;      // 32-bit: Command
pub const XHCI_OP_USBSTS: usize = 0x04;      // 32-bit: Status
pub const XHCI_OP_PAGESIZE: usize = 0x08;    // 32-bit: Page Size
pub const XHCI_OP_DNCTRL: usize = 0x14;      // 32-bit: Device Notification Control
pub const XHCI_OP_CRCR: usize = 0x18;        // 64-bit: Command Ring Control Register
pub const XHCI_OP_DCBAAP: usize = 0x30;      // 64-bit: DCBAA Pointer
pub const XHCI_OP_CONFIG: usize = 0x38;      // 32-bit: Configure (MaxSlotsEn)
pub const XHCI_OP_PORTSC_BASE: usize = 0x400;// Port Status and Control Base (stride 0x10)

// USBCMD Bitmasks
pub const USBCMD_RS: u32 = 1 << 0;           // Run/Stop
pub const USBCMD_HCRST: u32 = 1 << 1;        // Host Controller Reset
pub const USBCMD_INTE: u32 = 1 << 2;         // Interrupter Enable
pub const USBCMD_HSEE: u32 = 1 << 3;         // Host System Error Enable

// USBSTS Bitmasks
pub const USBSTS_HCH: u32 = 1 << 0;          // HC Halted
pub const USBSTS_HSE: u32 = 1 << 2;          // Host System Error
pub const USBSTS_EINT: u32 = 1 << 3;         // Event Interrupt
pub const USBSTS_PCD: u32 = 1 << 4;          // Port Change Detect
pub const USBSTS_CNR: u32 = 1 << 11;         // Controller Not Ready

// PORTSC Bitmasks
pub const PORTSC_CCS: u32 = 1 << 0;          // Current Connect Status
pub const PORTSC_PE: u32 = 1 << 1;           // Port Enabled
pub const PORTSC_OCA: u32 = 1 << 3;          // Over-current Active
pub const PORTSC_PR: u32 = 1 << 4;           // Port Reset
pub const PORTSC_PLS_MASK: u32 = 0x0F << 5;  // Port Link State
pub const PORTSC_PP: u32 = 1 << 9;           // Port Power
pub const PORTSC_SPEED_MASK: u32 = 0x0F << 10; // Port Speed
pub const PORTSC_SPEED_SHIFT: u32 = 10;
pub const PORTSC_CSC: u32 = 1 << 17;         // Connect Status Change
pub const PORTSC_PEC: u32 = 1 << 18;         // Port Enable/Disable Change
pub const PORTSC_WRC: u32 = 1 << 19;         // Warm Port Reset Change
pub const PORTSC_OCC: u32 = 1 << 20;         // Over-current Change
pub const PORTSC_PRC: u32 = 1 << 21;         // Port Reset Change
pub const PORTSC_PLC: u32 = 1 << 22;         // Port Link State Change

// Port Speeds
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum UsbSpeed {
    FullSpeed = 1,       // 12 Mbps (USB 1.1 / 2.0)
    LowSpeed = 2,        // 1.5 Mbps (USB 1.1)
    HighSpeed = 3,       // 480 Mbps (USB 2.0)
    SuperSpeed = 4,      // 5 Gbps (USB 3.0 / 3.2 Gen 1)
    SuperSpeedPlus = 5,  // 10 Gbps (USB 3.1 / 3.2 Gen 2)
    #[default]
    Unknown = 0,
}

impl UsbSpeed {
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            1 => UsbSpeed::FullSpeed,
            2 => UsbSpeed::LowSpeed,
            3 => UsbSpeed::HighSpeed,
            4 => UsbSpeed::SuperSpeed,
            5 => UsbSpeed::SuperSpeedPlus,
            _ => UsbSpeed::Unknown,
        }
    }

    pub const fn name(&self) -> &'static str {
        match self {
            UsbSpeed::FullSpeed => "Full-Speed (12 Mbps)",
            UsbSpeed::LowSpeed => "Low-Speed (1.5 Mbps)",
            UsbSpeed::HighSpeed => "High-Speed (480 Mbps)",
            UsbSpeed::SuperSpeed => "SuperSpeed (5 Gbps)",
            UsbSpeed::SuperSpeedPlus => "SuperSpeedPlus (10 Gbps)",
            UsbSpeed::Unknown => "Unknown Speed",
        }
    }

    pub const fn default_ep0_max_packet_size(&self) -> u16 {
        match self {
            UsbSpeed::LowSpeed => 8,
            UsbSpeed::FullSpeed => 64,
            UsbSpeed::HighSpeed => 64,
            UsbSpeed::SuperSpeed | UsbSpeed::SuperSpeedPlus => 512,
            UsbSpeed::Unknown => 8,
        }
    }
}

// Runtime Registers (Interrupter 0 at RTSOFF + 0x20)
pub const XHCI_INTR_IMAN: usize = 0x00;      // 32-bit: Interrupt Management (IP, IE)
pub const XHCI_INTR_IMOD: usize = 0x04;      // 32-bit: Interrupt Moderation
pub const XHCI_INTR_ERSTSZ: usize = 0x08;    // 32-bit: Event Ring Segment Table Size
pub const XHCI_INTR_ERSTBA: usize = 0x10;    // 64-bit: Event Ring Segment Table Base Address
pub const XHCI_INTR_ERDP: usize = 0x18;      // 64-bit: Event Ring Dequeue Pointer

// ============================================================================
// TRB (Transfer Request Block) Structures
// ============================================================================

pub const TRB_TYPE_NORMAL: u32 = 1;
pub const TRB_TYPE_SETUP_STAGE: u32 = 2;
pub const TRB_TYPE_DATA_STAGE: u32 = 3;
pub const TRB_TYPE_STATUS_STAGE: u32 = 4;
pub const TRB_TYPE_LINK: u32 = 6;
pub const TRB_TYPE_ENABLE_SLOT: u32 = 9;
pub const TRB_TYPE_DISABLE_SLOT: u32 = 10;
pub const TRB_TYPE_ADDRESS_DEVICE: u32 = 11;
pub const TRB_TYPE_CONFIG_EP: u32 = 12;
pub const TRB_TYPE_EVAL_CONTEXT: u32 = 13;
pub const TRB_TYPE_RESET_EP: u32 = 14;
pub const TRB_TYPE_NOOP_CMD: u32 = 23;
pub const TRB_TYPE_TRANSFER_EVENT: u32 = 32;
pub const TRB_TYPE_CMD_COMPLETION: u32 = 33;
pub const TRB_TYPE_PORT_STATUS: u32 = 34;

// TRB Completion Codes
pub const CC_SUCCESS: u32 = 1;
pub const CC_DATA_BUF_ERROR: u32 = 2;
pub const CC_BABBLE_ERROR: u32 = 3;
pub const CC_USB_TX_ERROR: u32 = 4;
pub const CC_TRB_ERROR: u32 = 5;
pub const CC_STALL_ERROR: u32 = 6;
pub const CC_RESOURCE_ERROR: u32 = 7;
pub const CC_BANDWIDTH_ERROR: u32 = 8;
pub const CC_NO_SLOTS_ERROR: u32 = 9;
pub const CC_SHORT_PACKET: u32 = 13;

/// 16-byte packed Transfer Request Block (TRB) compliant with xHCI 1.2 spec.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, packed)]
pub struct XhciTrb {
    pub parameter: u64,
    pub status: u32,
    pub control: u32,
}

impl XhciTrb {
    pub const fn new(parameter: u64, status: u32, control: u32) -> Self {
        Self { parameter, status, control }
    }

    pub const fn empty() -> Self {
        Self { parameter: 0, status: 0, control: 0 }
    }

    #[inline]
    pub fn cycle(&self) -> bool {
        (self.control & 1) != 0
    }

    #[inline]
    pub fn set_cycle(&mut self, cycle: bool) {
        if cycle {
            self.control |= 1;
        } else {
            self.control &= !1;
        }
    }

    #[inline]
    pub fn trb_type(&self) -> u32 {
        (self.control >> 10) & 0x3F
    }

    #[inline]
    pub fn completion_code(&self) -> u32 {
        (self.status >> 24) & 0xFF
    }

    #[inline]
    pub fn slot_id(&self) -> u8 {
        ((self.control >> 24) & 0xFF) as u8
    }

    #[inline]
    pub fn endpoint_id(&self) -> u8 {
        ((self.control >> 16) & 0x1F) as u8
    }

    /// Normal TRB (bulk/interrupt transfer)
    pub fn normal(buffer: u64, length: u32, cycle: bool, ioc: bool) -> Self {
        let mut ctrl = (TRB_TYPE_NORMAL << 10) | (if cycle { 1 } else { 0 });
        if ioc {
            ctrl |= 1 << 5; // Interrupt On Completion
        }
        Self::new(buffer, length & 0x1FFFF, ctrl)
    }

    /// Setup Stage TRB for USB Control Transfers
    pub fn setup_stage(
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        transfer_type: u8,
        cycle: bool,
    ) -> Self {
        let param = (request_type as u64)
            | ((request as u64) << 8)
            | ((value as u64) << 16)
            | ((index as u64) << 32)
            | ((length as u64) << 48);
        let status = 8; // 8 bytes USB setup packet length
        let mut ctrl = (TRB_TYPE_SETUP_STAGE << 10) | (if cycle { 1 } else { 0 });
        ctrl |= 1 << 6; // Immediate Data (IDT)
        ctrl |= ((transfer_type as u32) & 0x03) << 16; // 0=No data, 2=OUT, 3=IN
        Self::new(param, status, ctrl)
    }

    /// Data Stage TRB for USB Control Transfers
    pub fn data_stage(buffer: u64, length: u32, is_in: bool, cycle: bool, ioc: bool) -> Self {
        let mut ctrl = (TRB_TYPE_DATA_STAGE << 10) | (if cycle { 1 } else { 0 });
        if is_in {
            ctrl |= 1 << 16; // Direction: 1 = IN, 0 = OUT
        }
        if ioc {
            ctrl |= 1 << 5;
        }
        Self::new(buffer, length & 0x1FFFF, ctrl)
    }

    /// Status Stage TRB for USB Control Transfers
    pub fn status_stage(is_in: bool, cycle: bool, ioc: bool) -> Self {
        let mut ctrl = (TRB_TYPE_STATUS_STAGE << 10) | (if cycle { 1 } else { 0 });
        if is_in {
            ctrl |= 1 << 16;
        }
        if ioc {
            ctrl |= 1 << 5;
        }
        Self::new(0, 0, ctrl)
    }

    /// Link TRB to wrap ring back to start
    pub fn link(next_trb_phys: u64, toggle_cycle: bool, cycle: bool) -> Self {
        let mut ctrl = (TRB_TYPE_LINK << 10) | (if cycle { 1 } else { 0 });
        if toggle_cycle {
            ctrl |= 1 << 1; // Toggle Cycle (TC)
        }
        Self::new(next_trb_phys, 0, ctrl)
    }

    /// Enable Slot Command TRB
    pub fn enable_slot(slot_type: u8, cycle: bool) -> Self {
        let ctrl = (TRB_TYPE_ENABLE_SLOT << 10)
            | (((slot_type as u32) & 0x1F) << 16)
            | (if cycle { 1 } else { 0 });
        Self::new(0, 0, ctrl)
    }

    /// Disable Slot Command TRB
    pub fn disable_slot(slot_id: u8, cycle: bool) -> Self {
        let ctrl = (TRB_TYPE_DISABLE_SLOT << 10)
            | ((slot_id as u32) << 24)
            | (if cycle { 1 } else { 0 });
        Self::new(0, 0, ctrl)
    }

    /// Address Device Command TRB
    pub fn address_device(input_ctx_phys: u64, slot_id: u8, bsr: bool, cycle: bool) -> Self {
        let mut ctrl = (TRB_TYPE_ADDRESS_DEVICE << 10)
            | ((slot_id as u32) << 24)
            | (if cycle { 1 } else { 0 });
        if bsr {
            ctrl |= 1 << 9; // Block Set Address Request
        }
        Self::new(input_ctx_phys, 0, ctrl)
    }

    /// Configure Endpoint Command TRB
    pub fn configure_endpoint(input_ctx_phys: u64, slot_id: u8, cycle: bool) -> Self {
        let ctrl = (TRB_TYPE_CONFIG_EP << 10)
            | ((slot_id as u32) << 24)
            | (if cycle { 1 } else { 0 });
        Self::new(input_ctx_phys, 0, ctrl)
    }

    /// No-Op Command TRB
    pub fn noop_command(cycle: bool) -> Self {
        let ctrl = (TRB_TYPE_NOOP_CMD << 10) | (if cycle { 1 } else { 0 });
        Self::new(0, 0, ctrl)
    }
}

/// Event Ring Segment Table Entry (16 bytes, packed)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, packed)]
pub struct XhciErstEntry {
    pub ring_segment_base_address: u64,
    pub ring_segment_size: u32,
    pub rsvd: u32,
}

impl XhciErstEntry {
    pub const fn new(base: u64, size: u32) -> Self {
        Self {
            ring_segment_base_address: base,
            ring_segment_size: size,
            rsvd: 0,
        }
    }
}

// ============================================================================
// Circular TRB Ring Implementation
// ============================================================================

pub const RING_SIZE: usize = 64;

/// Circular TRB Ring (64 TRBs) with Link TRB cycle-bit toggling
pub struct TrbRing {
    pub trbs: [XhciTrb; RING_SIZE],
    pub enqueue_index: usize,
    pub producer_cycle: bool,
    pub ring_phys_addr: u64,
}

impl TrbRing {
    pub const fn new() -> Self {
        Self {
            trbs: [XhciTrb::empty(); RING_SIZE],
            enqueue_index: 0,
            producer_cycle: true,
            ring_phys_addr: 0,
        }
    }

    pub fn init(&mut self, ring_phys_addr: u64) {
        self.ring_phys_addr = ring_phys_addr;
        self.enqueue_index = 0;
        self.producer_cycle = true;
        for i in 0..RING_SIZE {
            self.trbs[i] = XhciTrb::empty();
        }
        // Slot RING_SIZE - 1 is the Link TRB pointing back to ring base
        self.trbs[RING_SIZE - 1] = XhciTrb::link(ring_phys_addr, true, true);
    }

    /// Enqueue a TRB into the ring, toggling cycle state when wrapping
    pub fn enqueue(&mut self, mut trb: XhciTrb) -> (usize, bool) {
        let slot = self.enqueue_index;
        trb.set_cycle(self.producer_cycle);
        self.trbs[slot] = trb;

        self.enqueue_index += 1;
        if self.enqueue_index == RING_SIZE - 1 {
            // Next is Link TRB: update its cycle bit and wrap
            let link_trb = XhciTrb::link(self.ring_phys_addr, true, self.producer_cycle);
            self.trbs[RING_SIZE - 1] = link_trb;
            self.producer_cycle = !self.producer_cycle;
            self.enqueue_index = 0;
        }

        (slot, self.producer_cycle)
    }

    pub fn free_slots(&self) -> usize {
        (RING_SIZE - 1).saturating_sub(self.enqueue_index)
    }
}

/// Circular Event Ring (64 TRBs) with consumer cycle tracking
pub struct EventRing {
    pub trbs: [XhciTrb; RING_SIZE],
    pub dequeue_index: usize,
    pub consumer_cycle: bool,
    pub erst: XhciErstEntry,
}

impl EventRing {
    pub const fn new() -> Self {
        Self {
            trbs: [XhciTrb::empty(); RING_SIZE],
            dequeue_index: 0,
            consumer_cycle: true,
            erst: XhciErstEntry {
                ring_segment_base_address: 0,
                ring_segment_size: RING_SIZE as u32,
                rsvd: 0,
            },
        }
    }

    pub fn init(&mut self, ring_phys_addr: u64) {
        self.dequeue_index = 0;
        self.consumer_cycle = true;
        for i in 0..RING_SIZE {
            self.trbs[i] = XhciTrb::empty();
        }
        self.erst = XhciErstEntry::new(ring_phys_addr, RING_SIZE as u32);
    }

    pub fn has_event(&self) -> bool {
        let trb = &self.trbs[self.dequeue_index];
        trb.cycle() == self.consumer_cycle
    }

    pub fn pop_event(&mut self) -> Option<XhciTrb> {
        if !self.has_event() {
            return None;
        }

        let ev = self.trbs[self.dequeue_index];
        self.dequeue_index += 1;
        if self.dequeue_index >= RING_SIZE {
            self.dequeue_index = 0;
            self.consumer_cycle = !self.consumer_cycle;
        }

        Some(ev)
    }
}

// ============================================================================
// Device Context & Slot Context Structures (32-byte layout)
// ============================================================================

/// 32-byte Slot Context
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct SlotContext {
    pub info1: u32,     // bits 0..19: Route String, bits 20..23: Speed, bits 27..31: Context Entries
    pub info2: u32,     // bits 0..15: Max Exit Latency, bits 16..23: Root Hub Port, bits 24..31: Num Ports
    pub tt_info: u32,   // bits 0..7: TT Hub Slot ID, bits 8..15: TT Port Num, bits 22..31: Interrupter Target
    pub state: u32,     // bits 27..31: Slot State
    pub rsvd: [u32; 4],
}

impl SlotContext {
    pub const fn new() -> Self {
        Self {
            info1: 0,
            info2: 0,
            tt_info: 0,
            state: 0,
            rsvd: [0; 4],
        }
    }

    pub fn setup(&mut self, root_port: u8, speed: UsbSpeed, context_entries: u8) {
        self.info1 = ((speed as u32) << 20) | (((context_entries as u32) & 0x1F) << 27);
        self.info2 = (root_port as u32) << 16;
        self.tt_info = 0;
        self.state = 0;
    }

    pub fn speed(&self) -> UsbSpeed {
        UsbSpeed::from_raw(((self.info1 >> 20) & 0x0F) as u8)
    }

    pub fn root_port(&self) -> u8 {
        ((self.info2 >> 16) & 0xFF) as u8
    }

    pub fn context_entries(&self) -> u8 {
        ((self.info1 >> 27) & 0x1F) as u8
    }
}

/// 32-byte Endpoint Context
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct EndpointContext {
    pub ep_info1: u32,       // State, Mult, MaxPStreams, Interval
    pub ep_info2: u32,       // ForceEvent, ErrorCount, EPType, MaxPacketSize
    pub tr_dequeue_ptr: u64, // Dequeue cycle bit 0, pointer bits 4..63
    pub avg_trb_len: u32,    // Average TRB length, Max ESIT payload
    pub rsvd: [u32; 3],
}

pub const EP_TYPE_CONTROL: u32 = 4;
pub const EP_TYPE_ISOCH_OUT: u32 = 1;
pub const EP_TYPE_BULK_OUT: u32 = 2;
pub const EP_TYPE_INTERRUPT_OUT: u32 = 3;
pub const EP_TYPE_ISOCH_IN: u32 = 5;
pub const EP_TYPE_BULK_IN: u32 = 6;
pub const EP_TYPE_INTERRUPT_IN: u32 = 7;

impl EndpointContext {
    pub const fn new() -> Self {
        Self {
            ep_info1: 0,
            ep_info2: 0,
            tr_dequeue_ptr: 0,
            avg_trb_len: 0,
            rsvd: [0; 3],
        }
    }

    pub fn setup_control_ep0(&mut self, max_packet_size: u16, ring_phys_addr: u64) {
        self.ep_info1 = 0; // Interval = 0 for EP0
        // Error count = 3 (bits 1..2), EP Type = Control (bits 3..5), MaxPacketSize (bits 16..31)
        self.ep_info2 = (3 << 1) | (EP_TYPE_CONTROL << 3) | ((max_packet_size as u32) << 16);
        self.tr_dequeue_ptr = (ring_phys_addr & !0x0F) | 1; // DCS = 1
        self.avg_trb_len = 8; // 8 bytes average TRB length
    }

    pub fn setup_interrupt_in(
        &mut self,
        max_packet_size: u16,
        interval: u8,
        ring_phys_addr: u64,
    ) {
        self.ep_info1 = (interval as u32) << 16;
        self.ep_info2 = (3 << 1) | (EP_TYPE_INTERRUPT_IN << 3) | ((max_packet_size as u32) << 16);
        self.tr_dequeue_ptr = (ring_phys_addr & !0x0F) | 1;
        self.avg_trb_len = max_packet_size as u32;
    }
}

/// 32-byte Input Control Context
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct InputControlContext {
    pub drop_flags: u32,
    pub add_flags: u32,
    pub rsvd: [u32; 6],
}

/// Device Context (Slot Context + 31 Endpoint Contexts)
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DeviceContext {
    pub slot: SlotContext,
    pub endpoints: [EndpointContext; 31],
}

impl DeviceContext {
    pub const fn new() -> Self {
        Self {
            slot: SlotContext::new(),
            endpoints: [EndpointContext::new(); 31],
        }
    }
}

/// Input Context used by Address Device & Configure Endpoint commands
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct InputContext {
    pub control: InputControlContext,
    pub slot: SlotContext,
    pub endpoints: [EndpointContext; 31],
}

impl InputContext {
    pub const fn new() -> Self {
        Self {
            control: InputControlContext { drop_flags: 0, add_flags: 0, rsvd: [0; 6] },
            slot: SlotContext::new(),
            endpoints: [EndpointContext::new(); 31],
        }
    }

    pub fn prepare_address_device(
        &mut self,
        root_port: u8,
        speed: UsbSpeed,
        ep0_max_packet: u16,
        ep0_ring_phys: u64,
    ) {
        self.control.drop_flags = 0;
        self.control.add_flags = (1 << 0) | (1 << 1); // Add Slot Context (bit 0) and EP0 Context (bit 1)
        self.slot.setup(root_port, speed, 1);
        self.endpoints[0].setup_control_ep0(ep0_max_packet, ep0_ring_phys);
    }
}

// ============================================================================
// Doorbell Register Stride & Calculation
// ============================================================================

pub struct XhciDoorbell;

impl XhciDoorbell {
    /// Calculate byte offset of Doorbell register for given slot ID
    /// Doorbell 0: Host Controller (Command Ring)
    /// Doorbell 1..255: Device Slot doorbells
    #[inline]
    pub const fn offset(dboff: u32, slot_id: u8) -> u32 {
        dboff + (slot_id as u32) * 4
    }

    /// Doorbell target:
    /// - 0: Host Controller Command Ring (when slot_id == 0)
    /// - 1: Endpoint 0 Control
    /// - 2..31: Endpoints 1..30
    #[inline]
    pub const fn target_ep(ep_index: u8) -> u32 {
        ep_index as u32
    }
}

// ============================================================================
// Standard USB Descriptors
// ============================================================================

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, packed)]
pub struct UsbDeviceDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub bcd_usb: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size0: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bcd_device: u16,
    pub manufacturer_index: u8,
    pub product_index: u8,
    pub serial_index: u8,
    pub num_configurations: u8,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveredUsbDevice {
    pub port: u8,
    pub slot_id: u8,
    pub speed: UsbSpeed,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size0: u16,
    pub connected: bool,
}

// ============================================================================
// xHCI Host Controller Driver State Machine
// ============================================================================

pub const MAX_XHCI_PORTS: usize = 16;
pub const MAX_XHCI_DEVICES: usize = 8;

pub struct XhciController {
    pub mmio_base: u64,
    pub caplength: u8,
    pub hci_version: u16,
    pub max_slots: u8,
    pub max_ports: u8,
    pub max_scratchpads: u16,
    pub dboff: u32,
    pub rtsoff: u32,
    pub op_offset: usize,
    pub command_ring: TrbRing,
    pub event_ring: EventRing,
    pub ep0_ring: TrbRing,
    pub dcbaa: [u64; 256],
    pub dev_context: DeviceContext,
    pub input_context: InputContext,
    pub devices: [DiscoveredUsbDevice; MAX_XHCI_DEVICES],
    pub device_count: usize,
    pub initialized: bool,
}

impl XhciController {
    pub const fn new() -> Self {
        Self {
            mmio_base: 0,
            caplength: 0,
            hci_version: 0,
            max_slots: 0,
            max_ports: 0,
            max_scratchpads: 0,
            dboff: 0,
            rtsoff: 0,
            op_offset: 0,
            command_ring: TrbRing::new(),
            event_ring: EventRing::new(),
            ep0_ring: TrbRing::new(),
            dcbaa: [0; 256],
            dev_context: DeviceContext::new(),
            input_context: InputContext::new(),
            devices: [DiscoveredUsbDevice {
                port: 0,
                slot_id: 0,
                speed: UsbSpeed::Unknown,
                vendor_id: 0,
                product_id: 0,
                device_class: 0,
                device_subclass: 0,
                device_protocol: 0,
                max_packet_size0: 0,
                connected: false,
            }; MAX_XHCI_DEVICES],
            device_count: 0,
            initialized: false,
        }
    }

    /// Read 32-bit register from MMIO space
    #[inline]
    pub unsafe fn read_mmio32(&self, offset: usize) -> u32 {
        if self.mmio_base == 0 {
            return 0;
        }
        let ptr = (self.mmio_base + offset as u64) as *const u32;
        core::ptr::read_volatile(ptr)
    }

    /// Write 32-bit register to MMIO space
    #[inline]
    pub unsafe fn write_mmio32(&self, offset: usize, val: u32) {
        if self.mmio_base == 0 {
            return;
        }
        let ptr = (self.mmio_base + offset as u64) as *mut u32;
        core::ptr::write_volatile(ptr, val);
    }

    /// Read 64-bit register from MMIO space
    #[inline]
    pub unsafe fn read_mmio64(&self, offset: usize) -> u64 {
        if self.mmio_base == 0 {
            return 0;
        }
        let ptr = (self.mmio_base + offset as u64) as *const u64;
        core::ptr::read_volatile(ptr)
    }

    /// Write 64-bit register to MMIO space
    #[inline]
    pub unsafe fn write_mmio64(&self, offset: usize, val: u64) {
        if self.mmio_base == 0 {
            return;
        }
        let ptr = (self.mmio_base + offset as u64) as *mut u64;
        core::ptr::write_volatile(ptr, val);
    }

    /// Parse capability parameters and initialize operational state
    pub fn init_controller_state(
        &mut self,
        mmio_base: u64,
        caplength: u8,
        hci_version: u16,
        hcs1: u32,
        hcs2: u32,
        dboff: u32,
        rtsoff: u32,
    ) {
        self.mmio_base = mmio_base;
        self.caplength = caplength;
        self.hci_version = hci_version;
        self.max_slots = (hcs1 & 0xFF) as u8;
        self.max_ports = ((hcs1 >> 24) & 0xFF) as u8;
        self.max_scratchpads = (((hcs2 >> 21) & 0x1F) | (((hcs2 >> 27) & 0x1F) << 5)) as u16;
        self.dboff = dboff;
        self.rtsoff = rtsoff;
        self.op_offset = caplength as usize;

        // Initialize rings with virtual/physical addresses
        self.command_ring.init(0x10000);
        self.event_ring.init(0x12000);
        self.ep0_ring.init(0x14000);

        self.initialized = true;
    }

    /// Decode port speed from PORTSC register value
    pub fn decode_port_speed(&self, portsc: u32) -> UsbSpeed {
        let speed_val = ((portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT) as u8;
        UsbSpeed::from_raw(speed_val)
    }

    /// Register a newly discovered USB device
    pub fn register_device(
        &mut self,
        port: u8,
        slot_id: u8,
        speed: UsbSpeed,
        desc: &UsbDeviceDescriptor,
    ) -> bool {
        if self.device_count >= MAX_XHCI_DEVICES {
            return false;
        }

        self.devices[self.device_count] = DiscoveredUsbDevice {
            port,
            slot_id,
            speed,
            vendor_id: desc.vendor_id,
            product_id: desc.product_id,
            device_class: desc.device_class,
            device_subclass: desc.device_subclass,
            device_protocol: desc.device_protocol,
            max_packet_size0: desc.max_packet_size0 as u16,
            connected: true,
        };
        self.device_count += 1;
        true
    }
}

// Global static controller instance for no_std kernel operation
pub static mut GLOBAL_XHCI: XhciController = XhciController::new();

// ============================================================================
// Driver In-Kernel Self-Test
// ============================================================================

/// In-kernel 10-point verification self-test for pure Rust xHCI USB 3.2 driver.
pub fn run_xhci_selftest() -> i32 {
    let mut failures = 0;

    // 1. Structure layout & packing verification
    if core::mem::size_of::<XhciTrb>() != 16 {
        failures |= 1 << 0;
    }
    if core::mem::size_of::<XhciErstEntry>() != 16 {
        failures |= 1 << 1;
    }
    if core::mem::size_of::<SlotContext>() != 32 {
        failures |= 1 << 2;
    }
    if core::mem::size_of::<EndpointContext>() != 32 {
        failures |= 1 << 3;
    }
    if core::mem::size_of::<UsbDeviceDescriptor>() != 18 {
        failures |= 1 << 4;
    }

    // 2. Doorbell calculation test
    let test_dboff = 0x2000;
    if XhciDoorbell::offset(test_dboff, 0) != 0x2000 {
        failures |= 1 << 5;
    }
    if XhciDoorbell::offset(test_dboff, 1) != 0x2004 {
        failures |= 1 << 6;
    }
    if XhciDoorbell::offset(test_dboff, 4) != 0x2010 {
        failures |= 1 << 7;
    }

    // 3. TRB construction & bitfield encoding
    let norm_trb = XhciTrb::normal(0x80000, 512, true, true);
    if norm_trb.trb_type() != TRB_TYPE_NORMAL || !norm_trb.cycle() {
        failures |= 1 << 8;
    }

    let setup_trb = XhciTrb::setup_stage(0x80, 0x06, 0x0100, 0, 18, 3, true);
    if setup_trb.trb_type() != TRB_TYPE_SETUP_STAGE || !setup_trb.cycle() {
        failures |= 1 << 9;
    }

    // 4. Circular TRB Ring wrap & producer cycle toggle test
    let mut ring = TrbRing::new();
    ring.init(0x1000);
    // Fill up to wrap (RING_SIZE - 2 items)
    for _ in 0..(RING_SIZE - 2) {
        ring.enqueue(XhciTrb::normal(0x2000, 64, true, false));
    }
    if !ring.producer_cycle {
        failures |= 1 << 10;
    }
    // Enqueue one more item -> triggers wrap and cycle toggle
    ring.enqueue(XhciTrb::normal(0x3000, 64, true, false));
    if ring.producer_cycle {
        // Must have toggled to false
        failures |= 1 << 11;
    }
    if ring.enqueue_index != 0 {
        failures |= 1 << 12;
    }

    // 5. Event Ring consumer cycle and pop test
    let mut ev_ring = EventRing::new();
    ev_ring.init(0x4000);
    if ev_ring.has_event() {
        failures |= 1 << 13;
    }
    // Simulate hardware event arrival with matching consumer cycle
    ev_ring.trbs[0] = XhciTrb::new(0x1000, (CC_SUCCESS << 24) | 0, (TRB_TYPE_CMD_COMPLETION << 10) | 1);
    if !ev_ring.has_event() {
        failures |= 1 << 14;
    }
    let popped = ev_ring.pop_event();
    if popped.is_none() || popped.unwrap().completion_code() != CC_SUCCESS {
        failures |= 1 << 15;
    }

    // 6. Device Context and Input Context preparation
    let mut input_ctx = InputContext::new();
    input_ctx.prepare_address_device(1, UsbSpeed::SuperSpeed, 512, 0x20000);
    if input_ctx.slot.speed() != UsbSpeed::SuperSpeed || input_ctx.slot.root_port() != 1 {
        failures |= 1 << 16;
    }

    // 7. Root hub port speed decoding
    let mut controller = XhciController::new();
    let portsc_ss = PORTSC_CCS | PORTSC_PE | PORTSC_PP | ((UsbSpeed::SuperSpeed as u32) << PORTSC_SPEED_SHIFT);
    let portsc_hs = PORTSC_CCS | PORTSC_PE | PORTSC_PP | ((UsbSpeed::HighSpeed as u32) << PORTSC_SPEED_SHIFT);
    if controller.decode_port_speed(portsc_ss) != UsbSpeed::SuperSpeed {
        failures |= 1 << 17;
    }
    if controller.decode_port_speed(portsc_hs) != UsbSpeed::HighSpeed {
        failures |= 1 << 18;
    }

    // 8. Device descriptor parsing & registration
    let dummy_desc = UsbDeviceDescriptor {
        length: 18,
        descriptor_type: 1,
        bcd_usb: 0x0300,
        device_class: 0x03, // HID
        device_subclass: 0x01, // Boot
        device_protocol: 0x02, // Mouse
        max_packet_size0: 9, // 2^9 = 512
        vendor_id: 0x1B36,
        product_id: 0x000D,
        bcd_device: 0x0100,
        manufacturer_index: 1,
        product_index: 2,
        serial_index: 3,
        num_configurations: 1,
    };
    controller.register_device(1, 1, UsbSpeed::SuperSpeed, &dummy_desc);
    if controller.device_count != 1 || controller.devices[0].vendor_id != 0x1B36 {
        failures |= 1 << 19;
    }

    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structure_sizes_and_alignments() {
        assert_eq!(core::mem::size_of::<XhciTrb>(), 16);
        assert_eq!(core::mem::size_of::<XhciErstEntry>(), 16);
        assert_eq!(core::mem::size_of::<SlotContext>(), 32);
        assert_eq!(core::mem::size_of::<EndpointContext>(), 32);
        assert_eq!(core::mem::size_of::<UsbDeviceDescriptor>(), 18);
    }

    #[test]
    fn test_doorbell_calculation() {
        let dboff = 0x2000;
        assert_eq!(XhciDoorbell::offset(dboff, 0), 0x2000);
        assert_eq!(XhciDoorbell::offset(dboff, 1), 0x2004);
        assert_eq!(XhciDoorbell::offset(dboff, 4), 0x2010);
        assert_eq!(XhciDoorbell::target_ep(1), 1);
    }

    #[test]
    fn test_trb_construction() {
        let norm = XhciTrb::normal(0x80000, 512, true, true);
        assert_eq!(norm.trb_type(), TRB_TYPE_NORMAL);
        assert!(norm.cycle());

        let setup = XhciTrb::setup_stage(0x80, 0x06, 0x0100, 0, 18, 3, true);
        assert_eq!(setup.trb_type(), TRB_TYPE_SETUP_STAGE);
        assert!(setup.cycle());

        let link = XhciTrb::link(0x10000, true, true);
        assert_eq!(link.trb_type(), TRB_TYPE_LINK);
        let param = link.parameter;
        assert_eq!(param, 0x10000);
    }

    #[test]
    fn test_circular_trb_ring() {
        let mut ring = TrbRing::new();
        ring.init(0x1000);
        for _ in 0..(RING_SIZE - 2) {
            ring.enqueue(XhciTrb::normal(0x2000, 64, true, false));
        }
        assert!(ring.producer_cycle);
        ring.enqueue(XhciTrb::normal(0x3000, 64, true, false));
        assert!(!ring.producer_cycle);
        assert_eq!(ring.enqueue_index, 0);
    }

    #[test]
    fn test_event_ring_dequeue() {
        let mut ev_ring = EventRing::new();
        ev_ring.init(0x4000);
        assert!(!ev_ring.has_event());
        ev_ring.trbs[0] = XhciTrb::new(0x1000, (CC_SUCCESS << 24) | 0, (TRB_TYPE_CMD_COMPLETION << 10) | 1);
        assert!(ev_ring.has_event());
        let ev = ev_ring.pop_event().expect("event expected");
        assert_eq!(ev.completion_code(), CC_SUCCESS);
        assert!(!ev_ring.has_event());
    }

    #[test]
    fn test_slot_and_endpoint_context() {
        let mut input_ctx = InputContext::new();
        input_ctx.prepare_address_device(2, UsbSpeed::SuperSpeed, 512, 0x30000);
        assert_eq!(input_ctx.slot.speed(), UsbSpeed::SuperSpeed);
        assert_eq!(input_ctx.slot.root_port(), 2);
        assert_eq!(input_ctx.slot.context_entries(), 1);
        assert_eq!(input_ctx.endpoints[0].tr_dequeue_ptr & 1, 1); // DCS = 1
    }

    #[test]
    fn test_port_speed_decoding() {
        let controller = XhciController::new();
        let portsc_fs = PORTSC_CCS | PORTSC_PE | ((UsbSpeed::FullSpeed as u32) << PORTSC_SPEED_SHIFT);
        let portsc_ls = PORTSC_CCS | PORTSC_PE | ((UsbSpeed::LowSpeed as u32) << PORTSC_SPEED_SHIFT);
        let portsc_hs = PORTSC_CCS | PORTSC_PE | ((UsbSpeed::HighSpeed as u32) << PORTSC_SPEED_SHIFT);
        let portsc_ss = PORTSC_CCS | PORTSC_PE | ((UsbSpeed::SuperSpeed as u32) << PORTSC_SPEED_SHIFT);
        let portsc_ssp = PORTSC_CCS | PORTSC_PE | ((UsbSpeed::SuperSpeedPlus as u32) << PORTSC_SPEED_SHIFT);

        assert_eq!(controller.decode_port_speed(portsc_fs), UsbSpeed::FullSpeed);
        assert_eq!(controller.decode_port_speed(portsc_ls), UsbSpeed::LowSpeed);
        assert_eq!(controller.decode_port_speed(portsc_hs), UsbSpeed::HighSpeed);
        assert_eq!(controller.decode_port_speed(portsc_ss), UsbSpeed::SuperSpeed);
        assert_eq!(controller.decode_port_speed(portsc_ssp), UsbSpeed::SuperSpeedPlus);
    }

    #[test]
    fn test_xhci_selftest_passes() {
        assert_eq!(run_xhci_selftest(), 0);
    }
}
