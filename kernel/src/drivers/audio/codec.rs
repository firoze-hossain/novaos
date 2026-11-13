//! kernel/src/drivers/audio/codec.rs - Intel High Definition Audio (HDA) Codec Subsystem
//!
//! NovaOS 2.0 (Month 5 - Desktop Experience): Pure Rust Intel HDA Audio Codec Driver.
//!
//! Compliant with Intel High Definition Audio Specification Rev 1.0a:
//!   - Codec addressing (CAD 0..14) over the high-speed serialized HDA link.
//!   - High-level verb builder and 64-bit response decoder (CORB/RIRB & Immediate Command).
//!   - Automatic topology discovery: Function Groups (AFG 0x01) and Audio Widgets.
//!   - Widget types: Audio Output (DAC), Audio Input (ADC), Mixer/Sum, Selector/Mux,
//!     Pin Complex, Power Widget, and Volume Knob.
//!   - Pin configuration default register parsing (Jack connectivity, device type, location, color).
//!   - EAPD (External Amplifier Power Down) and Headphone Amplifier controls.
//!   - Hardware amplifier gain/mute configuration with per-step dB attenuation.
//!   - High-fidelity format negotiation: 24-bit 96kHz, 24-bit 48kHz, 16-bit 44.1/48kHz PCM,
//!     with 1..8 channels (Mono, Stereo, 5.1 & 7.1 Surround).

#![allow(dead_code)]

// -----------------------------------------------------------------------------
// HDA Verb Opcodes and Constants
// -----------------------------------------------------------------------------

pub const HDA_MAX_CODECS: usize = 15;
pub const HDA_MAX_WIDGETS: usize = 64;

// 12-bit Parameter and Command Verbs
pub const VERB_GET_PARAMETER: u32 = 0xF00;
pub const VERB_GET_CONNECTION_SELECT: u32 = 0xF01;
pub const VERB_SET_CONNECTION_SELECT: u32 = 0x701;
pub const VERB_GET_CONNECTION_LIST: u32 = 0xF02;
pub const VERB_GET_PROCESSING_STATE: u32 = 0xF03;
pub const VERB_SET_PROCESSING_STATE: u32 = 0x703;
pub const VERB_GET_COEFF_INDEX: u32 = 0xD00;
pub const VERB_SET_COEFF_INDEX: u32 = 0x500;
pub const VERB_GET_PROCESSING_COEFF: u32 = 0xC00;
pub const VERB_SET_PROCESSING_COEFF: u32 = 0x400;
pub const VERB_GET_AMP_GAIN_MUTE: u32 = 0xB00;
pub const VERB_SET_AMP_GAIN_MUTE: u32 = 0x300;
pub const VERB_GET_CONV_FORMAT: u32 = 0xA00;
pub const VERB_SET_CONV_FORMAT: u32 = 0x200;
pub const VERB_GET_POWER_STATE: u32 = 0xF05;
pub const VERB_SET_POWER_STATE: u32 = 0x705;
pub const VERB_GET_CONV_STREAM_CHAN: u32 = 0xF06;
pub const VERB_SET_CONV_STREAM_CHAN: u32 = 0x706;
pub const VERB_GET_PIN_WIDGET_CONTROL: u32 = 0xF07;
pub const VERB_SET_PIN_WIDGET_CONTROL: u32 = 0x707;
pub const VERB_GET_UNSOLICITED_RESP: u32 = 0xF08;
pub const VERB_SET_UNSOLICITED_RESP: u32 = 0x708;
pub const VERB_GET_PIN_SENSE: u32 = 0xF09;
pub const VERB_EXEC_PIN_SENSE: u32 = 0x709;
pub const VERB_GET_EAPD_BTL: u32 = 0xF0C;
pub const VERB_SET_EAPD_BTL: u32 = 0x70C;
pub const VERB_GET_CONFIG_DEFAULT: u32 = 0xF1C;
pub const VERB_SET_CONFIG_DEFAULT_BYTE0: u32 = 0x71C;
pub const VERB_SET_CONFIG_DEFAULT_BYTE1: u32 = 0x71D;
pub const VERB_SET_CONFIG_DEFAULT_BYTE2: u32 = 0x71E;
pub const VERB_SET_CONFIG_DEFAULT_BYTE3: u32 = 0x71F;
pub const VERB_FUNCTION_RESET: u32 = 0x7FF;

// Parameter IDs for VERB_GET_PARAMETER (0xF00)
pub const PARAM_VENDOR_DEVICE_ID: u8 = 0x00;
pub const PARAM_REVISION_ID: u8 = 0x02;
pub const PARAM_SUB_NODE_COUNT: u8 = 0x04;
pub const PARAM_FUNCTION_GROUP_TYPE: u8 = 0x05;
pub const PARAM_AUDIO_FG_CAPS: u8 = 0x08;
pub const PARAM_AUDIO_WIDGET_CAPS: u8 = 0x09;
pub const PARAM_SUPPORTED_PCM_RATES: u8 = 0x0A;
pub const PARAM_SUPPORTED_STREAM_FORMATS: u8 = 0x0B;
pub const PARAM_PIN_CAPS: u8 = 0x0C;
pub const PARAM_INPUT_AMP_CAPS: u8 = 0x0D;
pub const PARAM_OUTPUT_AMP_CAPS: u8 = 0x12;
pub const PARAM_VOLUME_KNOB_CAPS: u8 = 0x13;

// Function Group Types
pub const FN_GROUP_AUDIO: u8 = 0x01;
pub const FN_GROUP_MODEM: u8 = 0x02;

// Audio Widget Types
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AudioWidgetType {
    AudioOutput = 0, // DAC Converter
    AudioInput = 1,  // ADC Converter
    AudioMixer = 2,  // Summing / Mixing Node
    AudioSelector = 3, // Multiplexer
    PinComplex = 4,  // Jack / Connector / Speaker / Mic
    PowerWidget = 5, // Power Domain Control
    VolumeKnob = 6,  // Hardware Volume Knob
    BeepGenerator = 7, // Beep tone generator
    VendorDefined = 15,
    Unknown = 255,
}

impl From<u8> for AudioWidgetType {
    fn from(val: u8) -> Self {
        match val {
            0 => AudioWidgetType::AudioOutput,
            1 => AudioWidgetType::AudioInput,
            2 => AudioWidgetType::AudioMixer,
            3 => AudioWidgetType::AudioSelector,
            4 => AudioWidgetType::PinComplex,
            5 => AudioWidgetType::PowerWidget,
            6 => AudioWidgetType::VolumeKnob,
            7 => AudioWidgetType::BeepGenerator,
            15 => AudioWidgetType::VendorDefined,
            _ => AudioWidgetType::Unknown,
        }
    }
}

// Power States
pub const POWER_STATE_D0: u32 = 0; // Fully operational
pub const POWER_STATE_D1: u32 = 1; // Light sleep
pub const POWER_STATE_D2: u32 = 2; // Deep sleep
pub const POWER_STATE_D3: u32 = 3; // Powered down

// Pin Widget Control Bits
pub const PIN_CTL_HEADPHONE_ENABLE: u8 = 1 << 7;
pub const PIN_CTL_OUTPUT_ENABLE: u8 = 1 << 6;
pub const PIN_CTL_INPUT_ENABLE: u8 = 1 << 5;
pub const PIN_CTL_VREF_HIZ: u8 = 0x00;
pub const PIN_CTL_VREF_50: u8 = 0x01;
pub const PIN_CTL_VREF_GROUND: u8 = 0x02;
pub const PIN_CTL_VREF_80: u8 = 0x04;
pub const PIN_CTL_VREF_100: u8 = 0x05;

// Pin Capabilities Bits (PARAM_PIN_CAPS)
pub const PIN_CAP_IMP_SENSE: u32 = 1 << 0;
pub const PIN_CAP_TRIGGER_REQ: u32 = 1 << 1;
pub const PIN_CAP_PRES_DETECT: u32 = 1 << 2;
pub const PIN_CAP_HP_DRV: u32 = 1 << 3;
pub const PIN_CAP_OUTPUT: u32 = 1 << 4;
pub const PIN_CAP_INPUT: u32 = 1 << 5;
pub const PIN_CAP_BALANCE: u32 = 1 << 6;
pub const PIN_CAP_HDMI: u32 = 1 << 7;
pub const PIN_CAP_VREF_MASK: u32 = 0xFF << 8;
pub const PIN_CAP_EAPD: u32 = 1 << 16;

// EAPD Bits
pub const EAPD_ENABLE: u32 = 1 << 1;
pub const EAPD_INVERT: u32 = 1 << 2;
pub const EAPD_LR_SWAP: u32 = 1 << 3;

// Default Pin Device Types (Parsed from Config Default)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PinDefaultDevice {
    LineOut = 0x00,
    Speaker = 0x01,
    Headphone = 0x02,
    Cd = 0x03,
    SpdifOut = 0x04,
    DigitalOtherOut = 0x05,
    ModemLine = 0x06,
    ModemHandset = 0x07,
    LineIn = 0x08,
    Aux = 0x09,
    Microphone = 0x0A,
    Telephony = 0x0B,
    SpdifIn = 0x0C,
    DigitalOtherIn = 0x0D,
    Other = 0x0F,
}

impl From<u8> for PinDefaultDevice {
    fn from(val: u8) -> Self {
        match val {
            0x00 => PinDefaultDevice::LineOut,
            0x01 => PinDefaultDevice::Speaker,
            0x02 => PinDefaultDevice::Headphone,
            0x03 => PinDefaultDevice::Cd,
            0x04 => PinDefaultDevice::SpdifOut,
            0x05 => PinDefaultDevice::DigitalOtherOut,
            0x06 => PinDefaultDevice::ModemLine,
            0x07 => PinDefaultDevice::ModemHandset,
            0x08 => PinDefaultDevice::LineIn,
            0x09 => PinDefaultDevice::Aux,
            0x0A => PinDefaultDevice::Microphone,
            0x0B => PinDefaultDevice::Telephony,
            0x0C => PinDefaultDevice::SpdifIn,
            0x0D => PinDefaultDevice::DigitalOtherIn,
            _ => PinDefaultDevice::Other,
        }
    }
}

// -----------------------------------------------------------------------------
// Pin Configuration Default Representation
// -----------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Default)]
pub struct PinConfigDefault {
    pub raw: u32,
    pub connectivity: u8, // 0 = Jack, 1 = None, 2 = Fixed, 3 = Both
    pub location: u8,     // 0x01 = Rear, 0x02 = Front, 0x03 = Left, etc.
    pub default_device: u8,
    pub connection_type: u8, // 0 = 1/8" stereo, 1 = 1/4" stereo, etc.
    pub color: u8,           // 1=Black, 2=Grey, 3=Blue, 4=Green, 5=Red, 9=Pink
    pub default_association: u8,
    pub sequence: u8,
}

impl PinConfigDefault {
    pub fn parse(raw: u32) -> Self {
        Self {
            raw,
            connectivity: ((raw >> 30) & 0x03) as u8,
            location: ((raw >> 24) & 0x3F) as u8,
            default_device: ((raw >> 20) & 0x0F) as u8,
            connection_type: ((raw >> 16) & 0x0F) as u8,
            color: ((raw >> 12) & 0x0F) as u8,
            default_association: ((raw >> 4) & 0x0F) as u8,
            sequence: (raw & 0x0F) as u8,
        }
    }

    pub fn device(&self) -> PinDefaultDevice {
        PinDefaultDevice::from(self.default_device)
    }

    pub fn is_output(&self) -> bool {
        matches!(
            self.device(),
            PinDefaultDevice::LineOut
                | PinDefaultDevice::Speaker
                | PinDefaultDevice::Headphone
                | PinDefaultDevice::SpdifOut
        )
    }

    pub fn is_input(&self) -> bool {
        matches!(
            self.device(),
            PinDefaultDevice::LineIn | PinDefaultDevice::Microphone | PinDefaultDevice::Aux
        )
    }
}

// -----------------------------------------------------------------------------
// Audio Widget Descriptor
// -----------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct HdaWidget {
    pub nid: u8,
    pub widget_type: AudioWidgetType,
    pub stereo: bool,
    pub in_amp_present: bool,
    pub out_amp_present: bool,
    pub amp_param_override: bool,
    pub format_override: bool,
    pub conn_list_len: u8,
    pub power_ctl: bool,
    pub l_r_swap: bool,
    pub pin_caps: u32,
    pub pin_config: PinConfigDefault,
    pub supported_rates: u32,
    pub supported_formats: u32,
    pub out_amp_steps: u8,
    pub out_amp_step_size: u8,
    pub out_amp_offset: u8,
    pub out_amp_mute_capable: bool,
    pub current_volume: u8,
    pub muted: bool,
    pub connections: [u8; 8],
    pub active: bool,
}

impl Default for HdaWidget {
    fn default() -> Self {
        Self {
            nid: 0,
            widget_type: AudioWidgetType::Unknown,
            stereo: true,
            in_amp_present: false,
            out_amp_present: false,
            amp_param_override: false,
            format_override: false,
            conn_list_len: 0,
            power_ctl: false,
            l_r_swap: false,
            pin_caps: 0,
            pin_config: PinConfigDefault::default(),
            supported_rates: 0,
            supported_formats: 0,
            out_amp_steps: 64,
            out_amp_step_size: 1, // 0.5 dB
            out_amp_offset: 40,
            out_amp_mute_capable: true,
            current_volume: 50,
            muted: false,
            connections: [0; 8],
            active: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Verb Builder & Decoder
// -----------------------------------------------------------------------------

/// Constructs a 32-bit HDA command verb word.
/// Format: [31:28 CAD] [27:20 NID] [19:8 Verb] [7:0 Payload]
pub const fn make_verb(cad: u8, nid: u8, verb: u32, payload: u8) -> u32 {
    let c = (cad as u32 & 0x0F) << 28;
    let n = (nid as u32 & 0xFF) << 20;
    let v = (verb & 0x0FFF) << 8;
    let p = payload as u32 & 0xFF;
    c | n | v | p
}

/// Constructs a 16-bit payload verb (such as format configuration or amp gain).
/// Format: [31:28 CAD] [27:20 NID] [19:16 4-bit Verb] [15:0 16-bit Payload]
pub const fn make_verb16(cad: u8, nid: u8, verb4: u32, payload16: u16) -> u32 {
    let c = (cad as u32 & 0x0F) << 28;
    let n = (nid as u32 & 0xFF) << 20;
    let v = (verb4 & 0x0F) << 16;
    let p = payload16 as u32;
    c | n | v | p
}

/// Unpacks a 64-bit RIRB response word.
/// Response: Bits 31:0 = Data, Bits 63:32 = ExResponse (CAD, Unsolicited, etc.)
pub fn unpack_rirb_response(raw: u64) -> (u32, u8, bool) {
    let data = (raw & 0xFFFF_FFFF) as u32;
    let ex = ((raw >> 32) & 0xFFFF_FFFF) as u32;
    let cad = (ex & 0x0F) as u8;
    let unsolicited = (ex & (1 << 4)) != 0;
    (data, cad, unsolicited)
}

// -----------------------------------------------------------------------------
// HdaCodec Structure & State Machine
// -----------------------------------------------------------------------------

pub struct HdaCodec {
    pub cad: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision_id: u32,
    pub afg_nid: u8,
    pub widgets_start_nid: u8,
    pub num_widgets: u8,
    pub widgets: [HdaWidget; HDA_MAX_WIDGETS],
    pub default_dac_nid: u8,
    pub default_adc_nid: u8,
    pub default_out_pin_nid: u8,
    pub default_in_pin_nid: u8,
    pub master_volume: u8, // 0..100%
    pub master_muted: bool,
    pub is_initialized: bool,
}

impl HdaCodec {
    pub const fn new(cad: u8) -> Self {
        Self {
            cad,
            vendor_id: 0,
            device_id: 0,
            revision_id: 0,
            afg_nid: 0,
            widgets_start_nid: 0,
            num_widgets: 0,
            widgets: [HdaWidget {
                nid: 0,
                widget_type: AudioWidgetType::Unknown,
                stereo: true,
                in_amp_present: false,
                out_amp_present: false,
                amp_param_override: false,
                format_override: false,
                conn_list_len: 0,
                power_ctl: false,
                l_r_swap: false,
                pin_caps: 0,
                pin_config: PinConfigDefault {
                    raw: 0,
                    connectivity: 0,
                    location: 0,
                    default_device: 0,
                    connection_type: 0,
                    color: 0,
                    default_association: 0,
                    sequence: 0,
                },
                supported_rates: 0,
                supported_formats: 0,
                out_amp_steps: 64,
                out_amp_step_size: 1,
                out_amp_offset: 40,
                out_amp_mute_capable: true,
                current_volume: 50,
                muted: false,
                connections: [0; 8],
                active: false,
            }; HDA_MAX_WIDGETS],
            default_dac_nid: 0,
            default_adc_nid: 0,
            default_out_pin_nid: 0,
            default_in_pin_nid: 0,
            master_volume: 85,
            master_muted: false,
            is_initialized: false,
        }
    }

    /// Initializes and provisions an audio codec using a command execution closure.
    pub fn init<F>(&mut self, mut exec_verb: F) -> Result<(), i32>
    where
        F: FnMut(u32) -> u32,
    {
        // 1. Query Root Node (NID 0) Vendor & Device ID
        let vid_did = exec_verb(make_verb(self.cad, 0, VERB_GET_PARAMETER, PARAM_VENDOR_DEVICE_ID));
        if vid_did == 0 || vid_did == 0xFFFF_FFFF {
            return Err(-1); // No response or unpopulated codec slot
        }
        self.vendor_id = (vid_did >> 16) as u16;
        self.device_id = (vid_did & 0xFFFF) as u16;

        // 2. Query Revision ID
        self.revision_id = exec_verb(make_verb(self.cad, 0, VERB_GET_PARAMETER, PARAM_REVISION_ID));

        // 3. Discover Function Groups under Root Node (NID 0)
        let fg_count_param = exec_verb(make_verb(self.cad, 0, VERB_GET_PARAMETER, PARAM_SUB_NODE_COUNT));
        let fg_start_nid = ((fg_count_param >> 16) & 0xFF) as u8;
        let num_fgs = (fg_count_param & 0xFF) as u8;

        for fg_idx in 0..num_fgs {
            let nid = fg_start_nid + fg_idx;
            let fg_type = exec_verb(make_verb(self.cad, nid, VERB_GET_PARAMETER, PARAM_FUNCTION_GROUP_TYPE));
            if (fg_type & 0xFF) as u8 == FN_GROUP_AUDIO {
                self.afg_nid = nid;
                break;
            }
        }

        if self.afg_nid == 0 {
            // Default to AFG NID 1 if not explicitly advertised
            self.afg_nid = 1;
        }

        // 4. Power up Audio Function Group to D0 (Active)
        exec_verb(make_verb(self.cad, self.afg_nid, VERB_SET_POWER_STATE, POWER_STATE_D0 as u8));

        // 5. Query Subordinate Audio Widgets inside the AFG
        let widget_count_param = exec_verb(make_verb(self.cad, self.afg_nid, VERB_GET_PARAMETER, PARAM_SUB_NODE_COUNT));
        self.widgets_start_nid = ((widget_count_param >> 16) & 0xFF) as u8;
        self.num_widgets = (widget_count_param & 0xFF) as u8;

        let count = (self.num_widgets as usize).min(HDA_MAX_WIDGETS);
        for i in 0..count {
            let w_nid = self.widgets_start_nid + i as u8;
            let w_caps = exec_verb(make_verb(self.cad, w_nid, VERB_GET_PARAMETER, PARAM_AUDIO_WIDGET_CAPS));
            let w_type = AudioWidgetType::from(((w_caps >> 20) & 0x0F) as u8);

            let mut widget = HdaWidget {
                nid: w_nid,
                widget_type: w_type,
                stereo: (w_caps & (1 << 0)) != 0,
                in_amp_present: (w_caps & (1 << 1)) != 0,
                out_amp_present: (w_caps & (1 << 2)) != 0,
                amp_param_override: (w_caps & (1 << 3)) != 0,
                format_override: (w_caps & (1 << 4)) != 0,
                conn_list_len: ((w_caps >> 8) & 0x7F) as u8,
                power_ctl: (w_caps & (1 << 10)) != 0,
                l_r_swap: (w_caps & (1 << 11)) != 0,
                pin_caps: 0,
                pin_config: PinConfigDefault::default(),
                supported_rates: 0,
                supported_formats: 0,
                out_amp_steps: 64,
                out_amp_step_size: 1,
                out_amp_offset: 40,
                out_amp_mute_capable: true,
                current_volume: self.master_volume,
                muted: false,
                connections: [0; 8],
                active: true,
            };

            // Wake widget up to D0
            if widget.power_ctl {
                exec_verb(make_verb(self.cad, w_nid, VERB_SET_POWER_STATE, POWER_STATE_D0 as u8));
            }

            // Inspect Pin Complex Specific Capabilities and Configuration
            if w_type == AudioWidgetType::PinComplex {
                widget.pin_caps = exec_verb(make_verb(self.cad, w_nid, VERB_GET_PARAMETER, PARAM_PIN_CAPS));
                let raw_cfg = exec_verb(make_verb(self.cad, w_nid, VERB_GET_CONFIG_DEFAULT, 0));
                widget.pin_config = PinConfigDefault::parse(raw_cfg);

                // Enable External Amplifier Power Down (EAPD) if available
                if (widget.pin_caps & PIN_CAP_EAPD) != 0 {
                    exec_verb(make_verb(self.cad, w_nid, VERB_SET_EAPD_BTL, EAPD_ENABLE as u8));
                }

                // If output capable pin, activate output control & headphone amp
                if (widget.pin_caps & PIN_CAP_OUTPUT) != 0 || widget.pin_config.is_output() {
                    let mut pin_ctrl = PIN_CTL_OUTPUT_ENABLE;
                    if (widget.pin_caps & PIN_CAP_HP_DRV) != 0 || widget.pin_config.device() == PinDefaultDevice::Headphone {
                        pin_ctrl |= PIN_CTL_HEADPHONE_ENABLE;
                    }
                    exec_verb(make_verb(self.cad, w_nid, VERB_SET_PIN_WIDGET_CONTROL, pin_ctrl));

                    if self.default_out_pin_nid == 0 {
                        self.default_out_pin_nid = w_nid;
                    }
                } else if (widget.pin_caps & PIN_CAP_INPUT) != 0 || widget.pin_config.is_input() {
                    // Activate input control for recording
                    exec_verb(make_verb(self.cad, w_nid, VERB_SET_PIN_WIDGET_CONTROL, PIN_CTL_INPUT_ENABLE | PIN_CTL_VREF_80));
                    if self.default_in_pin_nid == 0 {
                        self.default_in_pin_nid = w_nid;
                    }
                }
            }

            // Inspect Audio Output (DAC) Capabilities
            if w_type == AudioWidgetType::AudioOutput {
                widget.supported_rates = exec_verb(make_verb(self.cad, w_nid, VERB_GET_PARAMETER, PARAM_SUPPORTED_PCM_RATES));
                widget.supported_formats = exec_verb(make_verb(self.cad, w_nid, VERB_GET_PARAMETER, PARAM_SUPPORTED_STREAM_FORMATS));

                if self.default_dac_nid == 0 {
                    self.default_dac_nid = w_nid;
                }
            }

            // Inspect Audio Input (ADC) Capabilities
            if w_type == AudioWidgetType::AudioInput {
                widget.supported_rates = exec_verb(make_verb(self.cad, w_nid, VERB_GET_PARAMETER, PARAM_SUPPORTED_PCM_RATES));
                widget.supported_formats = exec_verb(make_verb(self.cad, w_nid, VERB_GET_PARAMETER, PARAM_SUPPORTED_STREAM_FORMATS));

                if self.default_adc_nid == 0 {
                    self.default_adc_nid = w_nid;
                }
            }

            // Inspect Amplifier Capabilities
            if widget.out_amp_present {
                let amp_caps = exec_verb(make_verb(self.cad, w_nid, VERB_GET_PARAMETER, PARAM_OUTPUT_AMP_CAPS));
                widget.out_amp_offset = (amp_caps & 0x7F) as u8;
                widget.out_amp_steps = ((amp_caps >> 8) & 0x7F) as u8;
                widget.out_amp_step_size = ((amp_caps >> 16) & 0x7F) as u8;
                widget.out_amp_mute_capable = (amp_caps & (1 << 31)) != 0;

                // Unmute and initialize volume to ~80%
                let step = (widget.out_amp_steps as u32 * self.master_volume as u32 / 100) as u16;
                // Set Output Amp Gain: [15: Output, 13: Left, 12: Right, 6:0 Gain Step]
                let amp_cmd = 0xB000 | (step & 0x7F);
                exec_verb(make_verb16(self.cad, w_nid, VERB_SET_AMP_GAIN_MUTE >> 8, amp_cmd));
            }

            self.widgets[i] = widget;
        }

        self.is_initialized = true;
        Ok(())
    }

    /// Sets playback stream format (e.g. 24-bit 96kHz or 48kHz PCM) on the Audio Output (DAC) widget.
    pub fn setup_playback_stream<F>(&mut self, stream_tag: u8, channel: u8, format_val: u16, mut exec_verb: F)
    where
        F: FnMut(u32) -> u32,
    {
        if self.default_dac_nid != 0 {
            // Set Converter Format on DAC
            exec_verb(make_verb16(self.cad, self.default_dac_nid, VERB_SET_CONV_FORMAT >> 8, format_val));

            // Assign Stream Tag & Channel: [7:4 Stream Tag, 3:0 Channel]
            let stream_chan = ((stream_tag & 0x0F) << 4) | (channel & 0x0F);
            exec_verb(make_verb(self.cad, self.default_dac_nid, VERB_SET_CONV_STREAM_CHAN, stream_chan));
        }
    }

    /// Sets recording stream format on the Audio Input (ADC) widget.
    pub fn setup_capture_stream<F>(&mut self, stream_tag: u8, channel: u8, format_val: u16, mut exec_verb: F)
    where
        F: FnMut(u32) -> u32,
    {
        if self.default_adc_nid != 0 {
            exec_verb(make_verb16(self.cad, self.default_adc_nid, VERB_SET_CONV_FORMAT >> 8, format_val));
            let stream_chan = ((stream_tag & 0x0F) << 4) | (channel & 0x0F);
            exec_verb(make_verb(self.cad, self.default_adc_nid, VERB_SET_CONV_STREAM_CHAN, stream_chan));
        }
    }

    /// Sets master output volume (0..100) and writes to active amplifier widgets.
    pub fn set_volume<F>(&mut self, volume_pct: u8, mut exec_verb: F)
    where
        F: FnMut(u32) -> u32,
    {
        let vol = volume_pct.min(100);
        self.master_volume = vol;

        for widget in self.widgets.iter_mut() {
            if widget.active && widget.out_amp_present {
                let steps = widget.out_amp_steps.max(1) as u32;
                let step = ((steps * vol as u32) / 100) as u16;
                let amp_cmd = if self.master_muted {
                    0xB080 // Mute bit set
                } else {
                    0xB000 | (step & 0x7F)
                };
                exec_verb(make_verb16(self.cad, widget.nid, VERB_SET_AMP_GAIN_MUTE >> 8, amp_cmd));
            }
        }
    }

    /// Toggles master mute.
    pub fn set_mute<F>(&mut self, mute: bool, exec_verb: F)
    where
        F: FnMut(u32) -> u32,
    {
        self.master_muted = mute;
        self.set_volume(self.master_volume, exec_verb);
    }
}
