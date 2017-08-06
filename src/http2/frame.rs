use std::iter;

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

#[derive(Debug, Clone, Eq, PartialEq)]
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
    Settings { ack: bool, settings: Vec<Setting> },
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
    Ping { ack: bool, opaque_data: u64 },
    // +-+-------------------------------------------------------------+
    // |R|                  Last-Stream-ID (31)                        |
    // +-+-------------------------------------------------------------+
    // |                      Error Code (32)                          |
    // +---------------------------------------------------------------+
    // |                  Additional Debug Data (*)                    |
    // +---------------------------------------------------------------+
    Goaway {
        last_stream_id: U31,
        error_code: ErrorCode,
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

impl FrameCodec {
    pub fn new() -> FrameCodec {
        Self::default()
    }
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
        let (_, identifier) = U31Codec.decode(src)?.unwrap();
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
                    let (e, stream_dpendency) = U31Codec.decode(&mut payload)?.unwrap();
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
                let (e, stream_dpendency) = U31Codec.decode(&mut payload)?.unwrap();
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
                if ack && !payload.is_empty() {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let n = payload.len() / 6;
                let mut settings = Vec::with_capacity(n);
                for _ in 0..n {
                    let setting = SettingCodec.decode(&mut payload)?.unwrap();
                    settings.push(setting);
                }
                Settings { ack, settings }
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
                let (_, promised_stream_id) = U31Codec.decode(&mut payload)?.unwrap();
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
                Ping { ack, opaque_data }
            }
            0x7 => {
                if identifier.as_u32() != 0x0 {
                    return Err(ErrorCode::ProtocolError.into());
                }
                if payload.len() < 8 {
                    return Err(ErrorCode::FrameSizeError.into());
                }
                let (_, last_stream_id) = U31Codec.decode(&mut payload)?.unwrap();
                let error_code = ErrorCodeCodec.decode(&mut payload)?.unwrap();
                let additional_debug_data = Vec::from(&payload as &[u8]);
                Goaway {
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
        use self::Frame::*;
        match item {
            Data {
                identifier,
                end_stream,
                pad_length,
                data,
            } => {
                let length = U24::from(
                    pad_length.map(|x| x + 1).unwrap_or_default() as u32 +
                        data.len() as u32,
                );
                U24Codec.encode(length, dst)?;
                dst.put_u8(0x0);
                let mut flags = 0x0;
                if end_stream {
                    flags |= 0x1;
                }
                if pad_length.is_some() {
                    flags |= 0x8;
                }
                dst.put_u8(flags);
                U31Codec.encode((false, identifier), dst)?;
                if let Some(pad_length) = pad_length {
                    dst.put_u8(pad_length);
                }
                dst.put_slice(&data);
                dst.extend(iter::repeat(0).take(
                    pad_length.unwrap_or_default() as usize,
                ));
            }
            Headers {
                identifier,
                end_stream,
                end_headers,
                pad_length,
                e,
                stream_dpendency,
                weight,
                header_block_fragment,
            } => {
                assert!(
                    e.is_some() == stream_dpendency.is_some() && e.is_some() == weight.is_some()
                );
                let length = U24::from(
                    pad_length.map(|x| x + 1).unwrap_or_default() as u32 +
                        if e.is_some() { 4 + 1 } else { 0 } +
                        header_block_fragment.len() as u32,
                );
                U24Codec.encode(length, dst)?;
                dst.put_u8(0x1);
                let mut flags = 0x0;
                if end_stream {
                    flags |= 0x1;
                }
                if end_headers {
                    flags |= 0x4;
                }
                if pad_length.is_some() {
                    flags |= 0x8;
                }
                if e.is_some() {
                    flags |= 0x20;
                }
                dst.put_u8(flags);
                U31Codec.encode((false, identifier), dst)?;
                if let Some(pad_length) = pad_length {
                    dst.put_u8(pad_length);
                }
                if let Some(e) = e {
                    U31Codec.encode((e, stream_dpendency.unwrap()), dst)?;
                    dst.put_u8(weight.unwrap());
                }
                dst.put_slice(&header_block_fragment);
                dst.extend(iter::repeat(0).take(
                    pad_length.unwrap_or_default() as usize,
                ));
            }
            Priority {
                identifier,
                e,
                stream_dpendency,
                weight,
            } => {
                U24Codec.encode(U24::from(5), dst)?;
                dst.put_u8(0x2);
                dst.put_u8(0x0);
                U31Codec.encode((false, identifier), dst)?;
                U31Codec.encode((e, stream_dpendency), dst)?;
                dst.put_u8(weight);
            }
            RstStream {
                identifier,
                error_code,
            } => {
                U24Codec.encode(U24::from(4), dst)?;
                dst.put_u8(0x3);
                dst.put_u8(0x0);
                U31Codec.encode((false, identifier), dst)?;
                ErrorCodeCodec.encode(error_code, dst)?;
            }
            Settings { ack, settings } => {
                assert!(!(ack && !settings.is_empty()));
                U24Codec.encode(U24::from(settings.len() as u32 * 6), dst)?;
                dst.put_u8(0x4);
                dst.put_u8(if ack { 0x1 } else { 0x0 });
                U31Codec.encode((false, U31::from(0)), dst)?;
                for setting in settings {
                    SettingCodec.encode(setting, dst)?;
                }
            }
            PushPromise {
                identifier,
                end_headers,
                pad_length,
                promised_stream_id,
                header_block_fragment,
            } => {
                let length = U24::from(
                    pad_length.map(|x| x + 1).unwrap_or_default() as u32 + 4 +
                        header_block_fragment.len() as u32,
                );
                U24Codec.encode(length, dst)?;
                dst.put_u8(0x5);
                let mut flags = 0x0;
                if end_headers {
                    flags |= 0x4;
                }
                if pad_length.is_some() {
                    flags |= 0x8;
                }
                dst.put_u8(flags);
                U31Codec.encode((false, identifier), dst)?;
                if let Some(pad_length) = pad_length {
                    dst.put_u8(pad_length);
                }
                U31Codec.encode((false, promised_stream_id), dst)?;
                dst.put_slice(&header_block_fragment);
                dst.extend(iter::repeat(0).take(
                    pad_length.unwrap_or_default() as usize,
                ));
            }
            Ping { ack, opaque_data } => {
                U24Codec.encode(U24::from(8), dst)?;
                dst.put_u8(0x6);
                dst.put_u8(if ack { 0x1 } else { 0x0 });
                U31Codec.encode((false, U31::from(0)), dst)?;
                dst.put_u64::<BigEndian>(opaque_data);
            }
            Goaway {
                last_stream_id,
                error_code,
                additional_debug_data,
            } => {
                let length = U24::from(4 + 4 + additional_debug_data.len() as u32);
                U24Codec.encode(length, dst)?;
                dst.put_u8(0x7);
                dst.put_u8(0x0);
                U31Codec.encode((false, U31::from(0)), dst)?;
                U31Codec.encode((false, last_stream_id), dst)?;
                ErrorCodeCodec.encode(error_code, dst)?;
                dst.put_slice(&additional_debug_data);
            }
            WindowUpdate {
                identifier,
                window_size_increment,
            } => {
                U24Codec.encode(U24::from(4), dst)?;
                dst.put_u8(0x8);
                dst.put_u8(0x0);
                U31Codec.encode((false, identifier), dst)?;
                U31Codec.encode((false, window_size_increment), dst)?;
            }
            Continuation {
                identifier,
                end_headers,
                header_block_fragment,
            } => {
                U24Codec.encode(
                    U24::from(header_block_fragment.len() as u32),
                    dst,
                )?;
                dst.put_u8(0x9);
                dst.put_u8(if end_headers { 0x4 } else { 0x0 });
                U31Codec.encode((false, identifier), dst)?;
                dst.put_slice(&header_block_fragment);
            }
            Unknown {
                identifier,
                kind,
                flags,
                payload,
            } => {
                U24Codec.encode(U24::from(payload.len() as u32), dst)?;
                dst.put_u8(kind);
                dst.put_u8(flags);
                U31Codec.encode((false, identifier), dst)?;
                dst.put_slice(&payload);
            }
        }
        Ok(())
    }
}

// +-------------------------------+
// |       Identifier (16)         |
// +-------------------------------+-------------------------------+
// |                        Value (32)                             |
// +---------------------------------------------------------------+
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn setting() {
        let tests: &[(&[u8], Setting)] =
            &[
                (&[0x0, 0x1, 0x0, 0x0, 0x0, 0x1], Setting::HeaderTableSize(1)),
                (&[0x0, 0x2, 0x0, 0x0, 0x0, 0x1], Setting::EnablePush(true)),
                (
                    &[0x0, 0x3, 0x0, 0x0, 0x0, 0x1],
                    Setting::MaxConcurrentStreams(1),
                ),
                (
                    &[0x0, 0x4, 0x0, 0x0, 0x0, 0x1],
                    Setting::InitialWindowSize(U31::from(1)),
                ),
                (
                    &[0x0, 0x5, 0x0, 0xff, 0xff, 0xff],
                    Setting::MaxFrameSize(U24::max_value()),
                ),
                (
                    &[0x0, 0x6, 0x0, 0x0, 0x0, 0x1],
                    Setting::MaxHeaderListSize(1),
                ),
                (&[0x0, 0x7, 0x0, 0x0, 0x0, 0x1], Setting::Unknown(0x7, 0x1)),
            ];
        for &(binary, result) in tests {
            let r = SettingCodec.decode(&mut binary.into()).unwrap().expect(
                "setting",
            );
            assert_eq!(r, result);
            let mut buf = BytesMut::with_capacity(binary.len());
            SettingCodec.encode(r, &mut buf).unwrap();
            assert_eq!(buf, binary);
        }
    }

    #[test]
    fn setting_error() {
        let tests: &[(&[u8], ErrorCode)] =
            &[
                (
                    &[0x0, 0x4, 0xff, 0xff, 0xff, 0xff],
                    ErrorCode::ProtocolError,
                ),
                (
                    &[0x0, 0x5, 0xff, 0xff, 0xff, 0xff],
                    ErrorCode::ProtocolError,
                ),
                (&[0x0, 0x5, 0x0, 0x0, 0x0, 0x0], ErrorCode::ProtocolError),
            ];
        for &(binary, error) in tests {
            let r = SettingCodec.decode(&mut binary.into()).expect_err(
                "setting_error",
            );
            assert!(if let Error::Http(e) = r {
                assert_eq!(e, error);
                true
            } else {
                false
            });
        }
    }

    #[test]
    fn eat_padding() {
        let tests: &[(&[u8], (u8, &[u8]))] = &[
            (&[0, 10, 10, 10], (0, &[10, 10, 10])),
            (&[2, 10, 0, 0], (2, &[10])),
        ];
        for &(binary, (padding, data)) in tests {
            let mut buf = BytesMut::from(binary);
            let r = super::eat_padding(&mut buf).unwrap();
            assert_eq!(r, padding);
            assert_eq!(&buf, data);
        }
    }

    #[test]
    fn eat_padding_error() {
        let tests: &[(&[u8], ErrorCode)] = &[
            (&[], ErrorCode::FrameSizeError),
            (&[1, 0], ErrorCode::ProtocolError),
            (&[5, 0], ErrorCode::ProtocolError),
        ];
        for &(binary, error) in tests {
            let mut buf = BytesMut::from(binary);
            let r = super::eat_padding(&mut buf).expect_err("eat_padding_error");
            assert!(if let Error::Http(e) = r {
                assert_eq!(e, error);
                true
            } else {
                false
            });
        }
    }

    #[test]
    fn frame() {
        let tests: Vec<(Vec<u8>, Frame)> =
            vec![
                (
                    vec![0, 0, 5, 0, 1 | 8, 0, 0, 0, 1, 2, 10, 10, 0, 0],
                    Frame::Data {
                        identifier: U31::from(1),
                        end_stream: true,
                        pad_length: Some(2),
                        data: vec![10, 10],
                    }
                ),
                (
                    vec![0, 0, 7, 1, 1 | 4 | 8 | 32, 0, 0, 0, 1, 0, 0, 0, 0, 5, 3, 10],
                    Frame::Headers {
                        identifier: U31::from(1),
                        end_stream: true,
                        end_headers: true,
                        pad_length: Some(0),
                        e: Some(false),
                        stream_dpendency: Some(U31::from(5)),
                        weight: Some(3),
                        header_block_fragment: vec![10],
                    }
                ),
                (
                    vec![0, 0, 5, 2, 0, 0, 0, 0, 1, 0, 0, 0, 5, 3],
                    Frame::Priority {
                        identifier: U31::from(1),
                        e: false,
                        stream_dpendency: U31::from(5),
                        weight: 3,
                    }
                ),
                (
                    vec![0, 0, 4, 3, 0, 0, 0, 0, 1, 0, 0, 0, 0],
                    Frame::RstStream {
                        identifier: U31::from(1),
                        error_code: ErrorCode::NoError,
                    }
                ),
                (
                    vec![0, 0, 0, 4, 1, 0, 0, 0, 0],
                    Frame::Settings {
                        ack: true,
                        settings: vec![],
                    }
                ),
                (
                    vec![0, 0, 6, 4, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0],
                    Frame::Settings {
                        ack: false,
                        settings: vec![Setting::HeaderTableSize(0)],
                    }
                ),
                (
                    vec![0, 0, 9, 5, 4 | 8, 0, 0, 0, 1, 2, 0, 0, 0, 5, 10, 10, 0, 0],
                    Frame::PushPromise {
                        identifier: U31::from(1),
                        end_headers: true,
                        pad_length: Some(2),
                        promised_stream_id: U31::from(5),
                        header_block_fragment: vec![10, 10],
                    }
                ),
                (
                    vec![0, 0, 8, 6, 1, 0, 0, 0, 0, 11, 11, 11, 11, 11, 11, 11, 11],
                    Frame::Ping {
                        ack: true,
                        opaque_data: 795741901218843403,
                    }
                ),
                (
                    vec![0, 0, 9, 7, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 10],
                    Frame::Goaway {
                        last_stream_id: U31::from(5),
                        error_code: ErrorCode::NoError,
                        additional_debug_data: vec![10],
                    }
                ),
                (
                    vec![0, 0, 4, 8, 0, 0, 0, 0, 1, 0, 0, 0, 6],
                    Frame::WindowUpdate {
                        identifier: U31::from(1),
                        window_size_increment: U31::from(6),
                    }
                ),
                (
                    vec![0, 0, 3, 9, 4, 0, 0, 0, 1, 10, 10, 10],
                    Frame::Continuation {
                        identifier: U31::from(1),
                        end_headers: true,
                        header_block_fragment: vec![10, 10, 10],
                    }
                ),
                (
                    vec![0, 0, 3, 10, 4, 0, 0, 0, 1, 10, 10, 10],
                    Frame::Unknown {
                        identifier: U31::from(1),
                        kind: 10,
                        flags: 4,
                        payload: vec![10, 10, 10],
                    }
                ),
            ];
        for (binary, result) in tests {
            let mut data: BytesMut = (&binary as &[u8]).into();
            let r = FrameCodec::new().decode(&mut data).unwrap().expect("frame");
            assert_eq!(data.len(), 0);
            assert_eq!(r, result);
            let mut buf = BytesMut::with_capacity(binary.len());
            FrameCodec::new().encode(r, &mut buf).unwrap();
            assert_eq!(buf, binary);
        }
    }
}
