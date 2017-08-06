use std::io;
use std::fmt;

use tokio_io::codec::{Encoder, Decoder};
use bytes::{BytesMut, BigEndian, ByteOrder, BufMut};

#[derive(Debug)]
pub enum Error {
    Http(ErrorCode),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use self::Error::*;
        match *self {
            Http(ref e) => e.fmt(f),
            Io(ref e) => e.fmt(f),
        }
    }
}

impl ::std::error::Error for Error {
    fn description(&self) -> &str {
        use self::Error::*;
        match *self {
            Http(_) => "HTTP Error",
            Io(ref e) => e.description(),
        }
    }

    fn cause(&self) -> Option<&::std::error::Error> {
        use self::Error::*;
        match *self {
            Http(_) => None,
            Io(ref e) => Some(e),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<ErrorCode> for Error {
    fn from(e: ErrorCode) -> Error {
        Error::Http(e)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ErrorCode {
    NoError,
    ProtocolError,
    InternalError,
    FlowControlError,
    SettingsTimeout,
    StreamClosed,
    FrameSizeError,
    RefusedStream,
    Cancel,
    CompressionError,
    ConnectError,
    EnhanceYourCalm,
    InadequateSecurity,
    Http11Required,
    Unknown(u32),
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use self::ErrorCode::*;
        let s = match *self {
            NoError => "NO_ERROR",
            ProtocolError => "PROTOCOL_ERROR",
            InternalError => "INTERNAL_ERROR",
            FlowControlError => "FLOW_CONTROL_ERROR",
            SettingsTimeout => "SETTINGS_TIMEOUT",
            StreamClosed => "STREAM_CLOSED",
            FrameSizeError => "FRAME_SIZE_ERROR",
            RefusedStream => "REFUSED_STREAM",
            Cancel => "CANCEL",
            CompressionError => "COMPRESSION_ERROR",
            ConnectError => "CONNECT_ERROR",
            EnhanceYourCalm => "ENHANCE_YOUR_CALM",
            InadequateSecurity => "INADEQUATE_SECURITY",
            Http11Required => "HTTP_1_1_REQUIRED",
            Unknown(c) => return write!(f, "UNKNOWN({:#x})", c),
        };
        f.write_str(s)
    }
}

pub struct ErrorCodeCodec;

impl Decoder for ErrorCodeCodec {
    type Item = ErrorCode;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        use self::ErrorCode::*;
        if src.len() < 4 {
            return Ok(None);
        }
        let code = BigEndian::read_u32(&src.split_to(4));
        let e = match code {
            0x0 => NoError,
            0x1 => ProtocolError,
            0x2 => InternalError,
            0x3 => FlowControlError,
            0x4 => SettingsTimeout,
            0x5 => StreamClosed,
            0x6 => FrameSizeError,
            0x7 => RefusedStream,
            0x8 => Cancel,
            0x9 => CompressionError,
            0xa => ConnectError,
            0xb => EnhanceYourCalm,
            0xc => InadequateSecurity,
            0xd => Http11Required,
            _ => Unknown(code),
        };
        Ok(Some(e))
    }
}

impl Encoder for ErrorCodeCodec {
    type Item = ErrorCode;
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        use self::ErrorCode::*;
        let code = match item {
            NoError => 0x0,
            ProtocolError => 0x1,
            InternalError => 0x2,
            FlowControlError => 0x3,
            SettingsTimeout => 0x4,
            StreamClosed => 0x5,
            FrameSizeError => 0x6,
            RefusedStream => 0x7,
            Cancel => 0x8,
            CompressionError => 0x9,
            ConnectError => 0xa,
            EnhanceYourCalm => 0xb,
            InadequateSecurity => 0xc,
            Http11Required => 0xd,
            Unknown(c) => c,
        };
        dst.put_u32::<BigEndian>(code);
        Ok(())
    }
}
