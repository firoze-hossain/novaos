//! kernel/src/drivers/audio/mod.rs
//!
//! NovaOS 2.0 (Month 5 - Desktop Experience): High-Definition Audio (HDA) Subsystem.

pub mod hda;
pub mod codec;

pub use hda::{
    run_hda_selftest, encode_stream_format, decode_stream_format,
    AudioBitDepth, AudioChannelMode, AudioSampleRate, HdaBdlEntry, HdaController,
    HdaStreamState, PcmParams, PcmStatus, GLOBAL_HDA_CONTROLLER,
    pcm_playback_write, pcm_capture_read, pcm_playback_ioctl,
    PCM_IOCTL_SET_PARAMS, PCM_IOCTL_GET_STATUS, PCM_IOCTL_START,
    PCM_IOCTL_DROP, PCM_IOCTL_DRAIN, PCM_IOCTL_SET_VOLUME, PCM_IOCTL_GET_VOLUME,
    PCI_CLASS_HDA,
};

pub use codec::{
    HdaCodec, HdaWidget, AudioWidgetType, PinConfigDefault, PinDefaultDevice,
    make_verb, make_verb16, unpack_rirb_response,
};
