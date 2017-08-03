use std::io;

use tokio_io::codec::{Encoder, Decoder};
use bytes::{BytesMut, BigEndian, ByteOrder, BufMut};

#[derive(Clone, Copy, Debug)]
pub struct U31(pub u32);

impl U31 {
    #[inline]
    pub fn min_value() -> U31 {
        U31(0)
    }
    #[inline]
    pub fn initial_value() -> U31 {
        U31(0xffff)
    }
    #[inline]
    pub fn max_value() -> U31 {
        U31(0x7fffffff)
    }
}

impl<'a> From<&'a [u8; 4]> for U31 {
    #[inline]
    fn from(src: &'a [u8; 4]) -> U31 {
        let n = BigEndian::read_u32(src);
        U31(n & 0x7fffffff)
    }
}

pub struct U31Codec;

impl Decoder for U31Codec {
    type Item = (bool, U31);
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            Ok(None)
        } else {
            let tmp = unsafe { &*(&src.split_to(4) as &[u8] as *const [u8] as *const [u8; 4]) };
            let b = (tmp[0] & 0x80) != 0;
            let n = U31::from(tmp);
            Ok(Some((b, n)))
        }
    }
}

impl Encoder for U31Codec {
    type Item = (bool, U31);
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let r = if item.0 {
            (item.1).0 | 0x80000000
        } else {
            (item.1).0
        };
        dst.put_u32::<BigEndian>(r);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct U24(pub u32);

impl U24 {
    #[inline]
    pub fn min_value() -> U24 {
        U24(0)
    }
    #[inline]
    pub fn initial_value() -> U24 {
        U24(0x4000)
    }
    #[inline]
    pub fn max_value() -> U24 {
        U24(0xffffff)
    }
}

impl<'a> From<&'a [u8; 3]> for U24 {
    #[inline]
    fn from(src: &'a [u8; 3]) -> U24 {
        let mut buf = [0u8; 4];
        buf[1..].clone_from_slice(src);
        U24(BigEndian::read_u32(&buf))
    }
}

pub struct U24Codec;

impl Decoder for U24Codec {
    type Item = U24;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 3 {
            Ok(None)
        } else {
            let tmp = unsafe { &*(&src.split_to(3) as &[u8] as *const [u8] as *const [u8; 3]) };
            let n = U24::from(tmp);
            Ok(Some(n))
        }
    }
}

impl Encoder for U24Codec {
    type Item = U24;
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let mut buf = [0u8; 4];
        BigEndian::write_u32(&mut buf, item.0);
        dst.extend_from_slice(&buf[1..]);
        Ok(())
    }
}
