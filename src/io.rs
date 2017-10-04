use std::rc::Rc;
use std::io::{self, Read, Write};
use std::net::Shutdown;

use futures::prelude::*;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::io::shutdown;
use bytes::{BytesMut, Bytes, BufMut, IntoBuf};
use h2::{self, server as h2s, client as h2c};
use tokio_core::net::TcpStream;

const BUF_SIZE: usize = 2048;

#[async]
pub fn copy_from_h2<
    W: AsyncWrite + 'static,
    B: Stream<Item = Bytes, Error = h2::Error> + 'static,
>(
    src: B,
    mut dst: W,
) -> Result<usize, h2::Error> {
    let mut counter = 0;
    #[async]
    for buf in src {
        let mut buf = buf.into_buf();
        let n = poll!(dst.write_buf(&mut buf))?;
        counter += n;
    }
    await!(shutdown(dst))?;
    Ok(counter)
}

pub trait SendData<B: IntoBuf> {
    fn send_data(&mut self, data: B, end_of_stream: bool) -> Result<(), h2::Error>;
}

impl<B: IntoBuf> SendData<B> for h2c::Stream<B> {
    fn send_data(&mut self, data: B, end_of_stream: bool) -> Result<(), h2::Error> {
        h2c::Stream::send_data(self, data, end_of_stream)
    }
}
impl<B: IntoBuf> SendData<B> for h2s::Stream<B> {
    fn send_data(&mut self, data: B, end_of_stream: bool) -> Result<(), h2::Error> {
        h2s::Stream::send_data(self, data, end_of_stream)
    }
}

#[async]
pub fn copy_to_h2<R: AsyncRead + 'static, H: SendData<Bytes> + 'static>(
    mut src: R,
    mut dst: H,
) -> Result<usize, h2::Error> {
    let mut counter = 0;
    let mut buf = BytesMut::with_capacity(BUF_SIZE);
    loop {
        let n = poll!(src.read_buf(&mut buf))?;
        if n == 0 {
            dst.send_data(buf.take().freeze(), true)?;
            break;
        } else {
            dst.send_data(buf.take().freeze(), false)?;
        }
        counter += n;
        let rem = buf.remaining_mut();
        if rem < BUF_SIZE {
            buf.reserve(BUF_SIZE - rem);
        }
    }
    Ok(counter)
}

// This is a custom type used to have a custom implementation of the
// `AsyncWrite::shutdown` method which actually calls `TcpStream::shutdown` to
// notify the remote end that we're done writing.
#[derive(Clone)]
pub struct Socket(Rc<TcpStream>);

impl Socket {
    pub fn new(s: TcpStream) -> Socket {
        Socket(Rc::new(s))
    }
}

impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self.0).read(buf)
    }
}

impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.0).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl AsyncRead for Socket {}

impl AsyncWrite for Socket {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        try!(self.0.shutdown(Shutdown::Write));
        Ok(().into())
    }
}

#[cfg(test)]
mod test {
    use std::io::Cursor;
    use std::rc::Rc;
    use std::cell::RefCell;

    use futures::prelude::*;
    use futures::stream::iter_ok;

    use super::*;

    #[derive(Clone)]
    struct Dst(Rc<RefCell<Cursor<Vec<u8>>>>);
    impl Write for Dst {
        fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> {
            self.0.borrow_mut().write(buf)
        }
        fn flush(&mut self) -> ::std::io::Result<()> {
            self.0.borrow_mut().flush()
        }
    }
    impl AsyncWrite for Dst {
        fn shutdown(&mut self) -> Poll<(), ::std::io::Error> {
            Ok(().into())
        }
    }
    impl SendData<Bytes> for Dst {
        fn send_data(&mut self, data: Bytes, _end_of_stream: bool) -> Result<(), h2::Error> {
            self.0.borrow_mut().get_mut().extend(data);
            Ok(())
        }
    }

    #[test]
    fn test_copy_from_h2() {
        let dst = Dst(Rc::new(RefCell::new(Cursor::new(Vec::new()))));
        let src = iter_ok(vec![
            Bytes::from("123"),
            Bytes::from("456"),
            Bytes::from("789"),
        ]);
        let r = copy_from_h2(src, dst.clone()).wait().unwrap();
        assert_eq!(r, 9);
        assert_eq!(dst.0.borrow().get_ref(), b"123456789");
    }

    #[test]
    fn test_copy_to_h2() {
        let dst = Dst(Rc::new(RefCell::new(Cursor::new(Vec::new()))));
        let src: &[u8] = b"123456789";
        let r = copy_to_h2(src, dst.clone()).wait().unwrap();
        assert_eq!(r, 9);
        assert_eq!(dst.0.borrow().get_ref(), b"123456789");
    }
}
