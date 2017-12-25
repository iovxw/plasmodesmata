use std::rc::Rc;
use std::io::{self, Read, Write};
use std::net::Shutdown;

use futures::prelude::*;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::io::shutdown;
use bytes::{Buf, BufMut, Bytes, BytesMut, IntoBuf};
use h2;
use tokio_core::net::TcpStream;

const BUF_SIZE: usize = 2048;

#[async]
pub fn copy_from_h2<W: AsyncWrite + 'static>(
    mut src: h2::RecvStream,
    mut dst: W,
) -> Result<usize, h2::Error> {
    let mut counter = 0;
    let mut rc_handle = src.release_capacity().clone();
    #[async]
    for data in src {
        rc_handle.release_capacity(data.len())?;
        let mut buf = data.into_buf();
        while buf.remaining() != 0 {
            let n = poll!(dst.write_buf(&mut buf))?;
            counter += n;
        }
    }
    await!(shutdown(dst))?;
    Ok(counter)
}

#[async]
pub fn copy_to_h2<R: AsyncRead + 'static>(
    mut src: R,
    mut dst: h2::SendStream<Bytes>,
) -> Result<usize, h2::Error> {
    let mut counter = 0;
    let mut buf = BytesMut::with_capacity(BUF_SIZE);
    loop {
        let n = poll!(src.read_buf(&mut buf))?;
        let end_of_stream = n == 0;
        let src_len = buf.len();
        dst.reserve_capacity(src_len);
        while dst.capacity() < src_len {
            poll!(dst.poll_capacity())?;
        }
        dst.send_data(buf.take().freeze(), end_of_stream)?;
        counter += n;
        if end_of_stream {
            break;
        }
        let rem = buf.remaining_mut();
        // make sure remaining size always >= BUF_SIZE
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
