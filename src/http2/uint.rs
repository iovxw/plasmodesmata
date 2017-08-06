use std::io;

use tokio_io::codec::{Encoder, Decoder};
use bytes::{BytesMut, BigEndian, ByteOrder, BufMut};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct U31(u32);

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
    #[inline]
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

impl<'a> From<&'a [u8; 4]> for U31 {
    #[inline]
    fn from(src: &'a [u8; 4]) -> U31 {
        let n = BigEndian::read_u32(src);
        U31(n & 0x7fffffff)
    }
}

impl From<u32> for U31 {
    #[inline]
    fn from(src: u32) -> U31 {
        if src > Self::max_value().as_u32() {
            Self::max_value()
        } else {
            U31(src)
        }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct U24(u32);

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
    #[inline]
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

impl<'a> From<&'a [u8; 3]> for U24 {
    #[inline]
    fn from(src: &'a [u8; 3]) -> U24 {
        let mut buf = [0u8; 4];
        buf[1..].copy_from_slice(src);
        U24(BigEndian::read_u32(&buf))
    }
}

impl From<u32> for U24 {
    #[inline]
    fn from(src: u32) -> U24 {
        if src > Self::max_value().as_u32() {
            Self::max_value()
        } else {
            U24(src)
        }
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn u31() {
        let data: &[u8] = &[255, 255, 255, 255];
        let r = U31Codec.decode(&mut data.into()).unwrap().expect("u31");
        assert_eq!(r, (true, U31::max_value()));
        let mut buf = BytesMut::with_capacity(4);
        U31Codec.encode(r, &mut buf).unwrap();
        assert_eq!(buf, data);
    }

    #[test]
    fn u24() {
        let data: &[u8] = &[255, 255, 255];
        let r = U24Codec.decode(&mut data.into()).unwrap().expect("u24");
        assert_eq!(r, U24::max_value());
        let mut buf = BytesMut::with_capacity(3);
        U24Codec.encode(r, &mut buf).unwrap();
        assert_eq!(buf, data);
    }
}
