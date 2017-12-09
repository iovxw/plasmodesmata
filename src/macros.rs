#[macro_export]
macro_rules! poll {
    ($e:expr) => ({
        loop {
            match $e {
                ::futures::__rt::Ok(::futures::Async::Ready(e)) => {
                    break ::futures::__rt::Ok(e)
                }
                ::futures::__rt::Ok(::futures::Async::NotReady) => {}
                ::futures::__rt::Err(e) => {
                    break ::futures::__rt::Err(e)
                }
            }
            yield ::futures::Async::NotReady;
        }
    })
}
