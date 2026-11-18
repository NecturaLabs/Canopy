//! The agent every request goes through: `ureq`'s own transports, except that a read the kernel
//! interrupted is read again.
//!
//! A blocking `recv` on a socket with a receive timeout (which `ureq` sets for every read) fails
//! with `EINTR` when the thread is woken for a signal, whether or not the signal's handler asks
//! for restarts, and neither the standard library nor `ureq` read again. Under load that turned a
//! complete response into a "network error" ("io: Interrupted system call"). No byte has been
//! consumed when it happens, so reading again is not a retry of the request.

use ureq::config::Config;
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};
use ureq::{Agent, Error};

/// An agent for `config` whose reads survive an interruption.
pub(crate) fn agent(config: Config) -> Agent {
    Agent::with_parts(
        config,
        RestartReads(DefaultConnector::new()),
        DefaultResolver::default(),
    )
}

/// An agent with `ureq`'s defaults (rustls with its bundled roots, the proxy environment
/// variables) whose reads survive an interruption, for Sentry's crash-report transport.
pub fn default_agent() -> Agent {
    agent(Agent::config_builder().build())
}

/// `ureq`'s default connector, each transport wrapped in [`Restarting`].
#[derive(Debug)]
struct RestartReads(DefaultConnector);

impl Connector for RestartReads {
    type Out = Box<dyn Transport>;

    fn connect(
        &self,
        details: &ConnectionDetails<'_>,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, Error> {
        let inner = self.0.connect(details, chained)?;
        Ok(inner.map(|transport| Restarting(transport).boxed()))
    }
}

/// A transport that asks again when waiting for input was interrupted.
#[derive(Debug)]
struct Restarting<T: Transport>(T);

impl<T: Transport> Transport for Restarting<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.0.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        self.0.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        loop {
            match self.0.await_input(timeout) {
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }

    fn is_open(&mut self) -> bool {
        self.0.is_open()
    }

    fn is_tls(&self) -> bool {
        self.0.is_tls()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use ureq::unversioned::transport::LazyBuffers;

    /// Fails its first `failures` reads with `kind`, then reports one byte.
    #[derive(Debug)]
    struct Failing {
        buffers: LazyBuffers,
        kind: ErrorKind,
        failures: u32,
        reads: u32,
    }

    impl Failing {
        fn new(kind: ErrorKind, failures: u32) -> Restarting<Self> {
            Restarting(Self {
                buffers: LazyBuffers::new(64, 64),
                kind,
                failures,
                reads: 0,
            })
        }
    }

    impl Transport for Failing {
        fn buffers(&mut self) -> &mut dyn Buffers {
            &mut self.buffers
        }

        fn transmit_output(&mut self, _: usize, _: NextTimeout) -> Result<(), Error> {
            Ok(())
        }

        fn await_input(&mut self, _: NextTimeout) -> Result<bool, Error> {
            self.reads += 1;
            if self.reads <= self.failures {
                return Err(Error::Io(self.kind.into()));
            }
            Ok(true)
        }

        fn is_open(&mut self) -> bool {
            true
        }
    }

    fn timeout() -> NextTimeout {
        NextTimeout::default()
    }

    #[test]
    fn an_interrupted_read_is_read_again() {
        let mut transport = Failing::new(ErrorKind::Interrupted, 3);
        assert!(transport.await_input(timeout()).unwrap());
        assert_eq!(transport.0.reads, 4);
    }

    #[test]
    fn any_other_failure_is_reported_at_once() {
        let mut transport = Failing::new(ErrorKind::ConnectionReset, 3);
        let error = transport.await_input(timeout()).unwrap_err();
        assert!(matches!(error, Error::Io(e) if e.kind() == ErrorKind::ConnectionReset));
        assert_eq!(transport.0.reads, 1);
    }
}
