use tokio_io::codec::{Encoder, Decoder};
use bytes::{BytesMut, BigEndian, ByteOrder, BufMut};

use super::uint::{U31, U31Codec, U24, U24Codec};
use super::error::{Error, ErrorCode, ErrorCodeCodec};

macro_rules! try_some {
    ($expr: expr) => {
        match $expr {
            Some(s) => s,
            None => return Ok(None),
        }
    }
}

pub enum Frame {
    // +---------------+
    // |Pad Length? (8)|
    // +---------------+-----------------------------------------------+
    // |                            Data (*)                         ...
    // +---------------------------------------------------------------+
    // |                           Padding (*)                       ...
    // +---------------------------------------------------------------+
    Data {
        identifier: U31,
        end_stream: bool,
        pad_length: Option<u8>,
        data: Vec<u8>,
    },
    // +---------------+
    // |Pad Length? (8)|
    // +-+-------------+-----------------------------------------------+
    // |E|                 Stream Dependency? (31)                     |
    // +-+-------------+-----------------------------------------------+
    // |  Weight? (8)  |
    // +-+-------------+-----------------------------------------------+
    // |                   Header Block Fragment (*)                 ...
    // +---------------------------------------------------------------+
    // |                           Padding (*)                       ...
    // +---------------------------------------------------------------+
    Headers {
        identifier: U31,
        end_stream: bool,
        end_headers: bool,
        pad_length: Option<u8>,
        e: Option<bool>, // priority flag
        stream_dpendency: Option<U31>, // priority flag
        weight: Option<u8>, // priority flag
        header_block_fragment: Vec<u8>, // TODO
    },
    // +-+-------------------------------------------------------------+
    // |E|                  Stream Dependency (31)                     |
    // +-+-------------+-----------------------------------------------+
    // |   Weight (8)  |
    // +-+-------------+
    Priority {
        identifier: U31,
        e: bool,
        stream_dpendency: U31,
        weight: u8,
    },
    // +---------------------------------------------------------------+
    // |                        Error Code (32)                        |
    // +---------------------------------------------------------------+
    RstStream {
        identifier: U31,
        error_code: ErrorCode,
    },
    // +-------------------------------+
    // |       Identifier (16)         |
    // +-------------------------------+-------------------------------+
    // |                        Value (32)                             |
    // +---------------------------------------------------------------+
    // ...
    Settings {
        identifier: U31,
        ack: bool,
        settings: Vec<Setting>,
    },
    // +---------------+
    // |Pad Length? (8)|
    // +-+-------------+-----------------------------------------------+
    // |R|                  Promised Stream ID (31)                    |
    // +-+-----------------------------+-------------------------------+
    // |                   Header Block Fragment (*)                 ...
    // +---------------------------------------------------------------+
    // |                           Padding (*)                       ...
    // +---------------------------------------------------------------+
    PushPromise {
        identifier: U31,
        end_headers: bool,
        pad_length: Option<u8>,
        promised_stream_id: U31,
        header_block_fragment: Vec<u8>,
    },
    // +---------------------------------------------------------------+
    // |                                                               |
    // |                      Opaque Data (64)                         |
    // |                                                               |
    // +---------------------------------------------------------------+
    Ping {
        identifier: U31,
        ack: bool,
        opaque_data: u64,
    },
    // +-+-------------------------------------------------------------+
    // |R|                  Last-Stream-ID (31)                        |
    // +-+-------------------------------------------------------------+
    // |                      Error Code (32)                          |
    // +---------------------------------------------------------------+
    // |                  Additional Debug Data (*)                    |
    // +---------------------------------------------------------------+
    Goaway {
        identifier: U31,
        last_stream_id: U31,
        error_code: u32,
        additional_debug_data: Vec<u8>,
    },
    // +-+-------------------------------------------------------------+
    // |R|              Window Size Increment (31)                     |
    // +-+-------------------------------------------------------------+
    WindowUpdate {
        identifier: U31,
        window_size_increment: U31,
    },
    // +---------------------------------------------------------------+
    // |                   Header Block Fragment (*)                 ...
    // +---------------------------------------------------------------+
    Continuation {
        identifier: U31,
        end_headers: bool,
        header_block_fragment: Vec<u8>,
    },
    Unknown {
        identifier: U31,
        kind: u8,
        flags: u8,
        payload: Vec<u8>,
    },
}

pub struct FrameCodec {
    length: Option<U24>,
}

impl Default for FrameCodec {
    fn default() -> Self {
        FrameCodec { length: None }
    }
}

impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = Error;

    // +-----------------------------------------------+
    // |                 Length (24)                   |
    // +---------------+---------------+---------------+
    // |   Type (8)    |   Flags (8)   |
    // +-+-------------+---------------+-------------------------------+
    // |R|                 Stream Identifier (31)                      |
    // +=+=============================================================+
    // |                   Frame Payload (0...)                      ...
    // +---------------------------------------------------------------+
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        use self::Frame::*;
        if self.length.is_none() {
            self.length = Some(try_some!(U24Codec.decode(src)?));
        }
        let payload_length = self.length.unwrap().as_u32() as usize;

        // frame header length = 9, `length` field length = 3, 9 - 3 = 6
        if src.len() < 6 + payload_length {
            return Ok(None);
        }

        let frame_type = src.split_to(1)[0];
        let flags = src.split_to(1)[0];
        let (_, identifier) = U31Codec.decode(&mut src.split_to(4))?.unwrap();
        let mut payload = src.split_to(payload_length);
        let frame = match frame_type {
            0x0 => {
                if identifier.as_u32() == 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                let end_stream = flags & 0x1 != 0;
                let pad_length = if flags & 0x8 != 0 {
                    let length = eat_padding(&mut payload)?;
                    Some(length)
                } else {
                    None
                };
                let data = Vec::from(&payload as &[u8]);
                Data {
                    identifier,
                    end_stream,
                    pad_length,
                    data,
                }
            }
            0x1 => {
                if identifier.as_u32() == 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                let end_stream = flags & 0x1 != 0;
                let end_headers = flags & 0x4 != 0;
                let pad_length = if flags & 0x8 != 0 {
                    let length = eat_padding(&mut payload)?;
                    Some(length)
                } else {
                    None
                };
                let (e, stream_dpendency, weight) = if flags & 0x20 != 0 {
                    if payload.len() < 5 {
                        return Err(ErrorCode::FrameSizeError.into());
                    }
                    let (e, stream_dpendency) = U31Codec.decode(&mut payload.split_to(4))?.unwrap();
                    let weight = payload.split_to(1)[0];
                    (Some(e), Some(stream_dpendency), Some(weight))
                } else {
                    (None, None, None)
                };
                let header_block_fragment = Vec::from(&payload as &[u8]);
                Headers {
                    identifier,
                    end_stream,
                    end_headers,
                    pad_length,
                    e,
                    stream_dpendency,
                    weight,
                    header_block_fragment,
                }
            }
            0x2 => {
                if identifier.as_u32() == 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                if payload.len() != 5 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let (e, stream_dpendency) = U31Codec.decode(&mut payload.split_to(4))?.unwrap();
                let weight = payload[0];
                Priority {
                    identifier,
                    e,
                    stream_dpendency,
                    weight,
                }
            }
            0x3 => {
                if identifier.as_u32() == 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                if payload.len() != 4 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let error_code = ErrorCodeCodec.decode(&mut payload)?.unwrap();
                RstStream {
                    identifier,
                    error_code,
                }
            }
            0x4 => {
                if identifier.as_u32() != 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                if payload.len() % 6 != 0 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let ack = flags & 0x1 != 0;
                if ack && payload.len() != 0 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let n = payload.len() / 6;
                let mut settings = Vec::with_capacity(n);
                for _ in 0..n {
                    let setting = SettingCodec.decode(src)?.unwrap();
                    settings.push(setting);
                }
                Settings {
                    identifier,
                    ack,
                    settings,
                }
            }
            0x5 => {
                if identifier.as_u32() == 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                let end_headers = flags & 0x4 != 0;
                let pad_length = if flags & 0x8 != 0 {
                    let length = eat_padding(&mut payload)?;
                    Some(length)
                } else {
                    None
                };
                if payload.len() < 4 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let (_, promised_stream_id) = U31Codec.decode(&mut payload.split_to(4))?.unwrap();
                let header_block_fragment = Vec::from(&payload as &[u8]);
                PushPromise {
                    identifier,
                    end_headers,
                    pad_length,
                    promised_stream_id,
                    header_block_fragment,
                }
            }
            0x6 => {
                if identifier.as_u32() != 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                if payload.len() != 8 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let ack = flags & 0x1 != 0;
                let opaque_data = BigEndian::read_u64(&payload);
                Ping {
                    identifier,
                    ack,
                    opaque_data,
                }
            }
            0x7 => {
                if identifier.as_u32() != 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                if payload.len() < 8 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let (_, last_stream_id) = U31Codec.decode(&mut payload.split_to(4))?.unwrap();
                let error_code = BigEndian::read_u32(&payload.split_to(4));
                let additional_debug_data = Vec::from(&payload as &[u8]);
                Goaway {
                    identifier,
                    last_stream_id,
                    error_code,
                    additional_debug_data,
                }
            }
            0x8 => {
                if payload_length != 4 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let (_, window_size_increment) = U31Codec.decode(&mut payload)?.unwrap();
                if window_size_increment.as_u32() == 0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                WindowUpdate {
                    identifier,
                    window_size_increment,
                }
            }
            0x9 => {
                if identifier.as_u32() == 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                let end_headers = flags & 0x4 != 0;
                let header_block_fragment = Vec::from(&payload as &[u8]);
                Continuation {
                    identifier,
                    end_headers,
                    header_block_fragment,
                }
            }
            _ => Unknown {
                identifier: identifier,
                kind: frame_type,
                flags: flags,
                payload: Vec::from(&payload as &[u8]),
            },
        };
        Ok(Some(frame))
    }
}

fn eat_padding(payload: &mut BytesMut) -> Result<u8, Error> {
    if payload.len() < 1 {
        return Err(ErrorCode::FrameSizeError.into());
    }
    let length = payload.split_to(1)[0];
    if payload.len() <= length as usize {
        return Err(ErrorCode::ProtocolError.into());
    }
    let payload_len = payload.len(); // avoid mutable borrow checker
    payload.split_off(payload_len - length as usize);
    Ok(length)
}

impl Encoder for FrameCodec {
    type Item = Frame;
    type Error = Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        unimplemented!()
    }
}

// +-------------------------------+
// |       Identifier (16)         |
// +-------------------------------+-------------------------------+
// |                        Value (32)                             |
// +---------------------------------------------------------------+
pub enum Setting {
    HeaderTableSize(u32),
    EnablePush(bool),
    MaxConcurrentStreams(u32),
    InitialWindowSize(U31),
    MaxFrameSize(U24),
    MaxHeaderListSize(u32),
    Unknown(u16, u32),
}

struct SettingCodec;

impl Decoder for SettingCodec {
    type Item = Setting;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        use self::Setting::*;
        if src.len() < 6 {
            return Ok(None);
        }
        let setting_id = BigEndian::read_u16(&src.split_to(2));
        let setting_value = BigEndian::read_u32(&src.split_to(4));
        let r = match setting_id {
            0x1 => HeaderTableSize(setting_value),
            0x2 => EnablePush(setting_value != 0),
            0x3 => MaxConcurrentStreams(setting_value),
            0x4 => {
                if setting_value > U31::max_value().as_u32() {
                    return Err(ErrorCode::ProtocolError.into());
                }
                InitialWindowSize(U31::from(setting_value))
            }
            0x5 => {
                if setting_value < U24::initial_value().as_u32() ||
                    setting_value > U24::max_value().as_u32()
                {
                    return Err(ErrorCode::ProtocolError.into());
                }
                MaxFrameSize(U24::from(setting_value))
            }
            0x6 => MaxHeaderListSize(setting_value),
            _ => Unknown(setting_id, setting_value),
        };
        Ok(Some(r))
    }
}

impl Encoder for SettingCodec {
    type Item = Setting;
    type Error = Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        use self::Setting::*;
        let (id, value) = match item {
            HeaderTableSize(value) => (0x1, value),
            EnablePush(value) => (0x2, if value { 0x1 } else { 0x0 }),
            MaxConcurrentStreams(value) => (0x3, value),
            InitialWindowSize(value) => (0x4, value.as_u32()),
            MaxFrameSize(value) => (0x5, value.as_u32()),
            MaxHeaderListSize(value) => (0x6, value),
            Unknown(id, value) => (id, value),
        };
        dst.put_u16::<BigEndian>(id);
        dst.put_u32::<BigEndian>(value);
        Ok(())
    }
}
