// Portions of this file are Copyright 2014 The Rust Project Developers.
// See http://rust-lang.org/COPYRIGHT.

use libc;
use {Errno, Result};
use std::mem;
use std::ptr;

pub use libc::{
    SIGHUP,
    SIGINT,
    SIGQUIT,
    SIGILL,
    SIGTRAP,
    SIGABRT,
    SIGBUS,
    SIGFPE,
    SIGKILL,
    SIGUSR1,
    SIGSEGV,
    SIGUSR2,
    SIGPIPE,
    SIGALRM,
    SIGTERM,
    SIGCHLD,
    SIGCONT,
    SIGSTOP,
    SIGTSTP,
    SIGTTIN,
    SIGTTOU,
    SIGURG,
    SIGXCPU,
    SIGXFSZ,
    SIGVTALRM,
    SIGPROF,
    SIGWINCH,
    SIGIO,
    SIGSYS,
};

#[cfg(target_os = "macos")]
pub use libc::{
    SIGEMT,
    SIGINFO,
};

#[cfg(not(target_os = "macos"))]
pub use libc::{
    SIGPWR,
    SIGSTKFLT,
    SIGIOT, // Alias for SIGABRT
    SIGPOLL, // Alias for SIGIO
    SIGUNUSED, // Alias for 31
};

pub const NSIG: libc::c_int = 32;

bitflags!{
    flags SaFlags: libc::c_int {
        const SA_NOCLDSTOP = libc::SA_NOCLDSTOP,
        const SA_NOCLDWAIT = libc::SA_NOCLDWAIT,
        const SA_NODEFER   = libc::SA_NODEFER,
        const SA_ONSTACK   = libc::SA_ONSTACK,
        const SA_RESETHAND = libc::SA_RESETHAND,
        const SA_RESTART   = libc::SA_RESTART,
        const SA_SIGINFO   = libc::SA_SIGINFO,
    }
}

bitflags!{
    flags SigFlags: libc::c_int {
        const SIG_BLOCK   = libc::SIG_BLOCK,
        const SIG_UNBLOCK = libc::SIG_UNBLOCK,
        const SIG_SETMASK = libc::SIG_SETMASK,
    }
}

#[derive(Clone, Copy)]
pub struct SigSet {
    sigset: libc::sigset_t
}

pub type SigNum = libc::c_int;

impl SigSet {
    pub fn all() -> SigSet {
        let mut sigset: libc::sigset_t = unsafe { mem::uninitialized() };
        let _ = unsafe { libc::sigfillset(&mut sigset as *mut libc::sigset_t) };

        SigSet { sigset: sigset }
    }

    pub fn empty() -> SigSet {
        let mut sigset: libc::sigset_t = unsafe { mem::uninitialized() };
        let _ = unsafe { libc::sigemptyset(&mut sigset as *mut libc::sigset_t) };

        SigSet { sigset: sigset }
    }

    pub fn add(&mut self, signum: SigNum) -> Result<()> {
        let res = unsafe { libc::sigaddset(&mut self.sigset as *mut libc::sigset_t, signum) };

        Errno::result(res).map(drop)
    }

    pub fn clear(&mut self) -> Result<()> {
        let res = unsafe { libc::sigemptyset(&mut self.sigset as *mut libc::sigset_t) };

        Errno::result(res).map(drop)
    }

    pub fn remove(&mut self, signum: SigNum) -> Result<()> {
        let res = unsafe { libc::sigdelset(&mut self.sigset as *mut libc::sigset_t, signum) };

        Errno::result(res).map(drop)
    }

    pub fn extend(&mut self, other: &SigSet) -> Result<()> {
        for i in 1..NSIG {
            if try!(other.contains(i)) {
                try!(self.add(i));
            }
        }
        Ok(())
    }

    pub fn contains(&self, signum: SigNum) -> Result<bool> {
        let res = unsafe { libc::sigismember(&self.sigset as *const libc::sigset_t, signum) };

        match try!(Errno::result(res)) {
            1 => Ok(true),
            0 => Ok(false),
            _ => unreachable!("unexpected value from sigismember"),
        }
    }

    /// Gets the currently blocked (masked) set of signals for the calling thread.
    pub fn thread_get_mask() -> Result<SigSet> {
        let mut oldmask: SigSet = unsafe { mem::uninitialized() };
        try!(pthread_sigmask(SigFlags::empty(), None, Some(&mut oldmask)));
        Ok(oldmask)
    }

    /// Sets the set of signals as the signal mask for the calling thread.
    pub fn thread_set_mask(&self) -> Result<()> {
        pthread_sigmask(SIG_SETMASK, Some(self), None)
    }

    /// Adds the set of signals to the signal mask for the calling thread.
    pub fn thread_block(&self) -> Result<()> {
        pthread_sigmask(SIG_BLOCK, Some(self), None)
    }

    /// Removes the set of signals from the signal mask for the calling thread.
    pub fn thread_unblock(&self) -> Result<()> {
        pthread_sigmask(SIG_UNBLOCK, Some(self), None)
    }

    /// Sets the set of signals as the signal mask, and returns the old mask.
    pub fn thread_swap_mask(&self, how: SigFlags) -> Result<SigSet> {
        let mut oldmask: SigSet = unsafe { mem::uninitialized() };
        try!(pthread_sigmask(how, Some(self), Some(&mut oldmask)));
        Ok(oldmask)
    }

    /// Suspends execution of the calling thread until one of the signals in the
    /// signal mask becomes pending, and returns the accepted signal.
    pub fn wait(&self) -> Result<SigNum> {
        let mut signum: SigNum = unsafe { mem::uninitialized() };
        let res = unsafe { libc::sigwait(&self.sigset as *const libc::sigset_t, &mut signum) };

        Errno::result(res).map(|_| signum)
    }
}

impl AsRef<libc::sigset_t> for SigSet {
    fn as_ref(&self) -> &libc::sigset_t {
        &self.sigset
    }
}

#[allow(unknown_lints)]
#[cfg_attr(not(raw_pointer_derive_allowed), allow(raw_pointer_derive))]
#[derive(Clone, Copy, PartialEq)]
pub enum SigHandler {
    SigDfl,
    SigIgn,
    Handler(extern fn(SigNum)),
    SigAction(extern fn(SigNum, *mut libc::siginfo_t, *mut libc::c_void))
}

pub struct SigAction {
    sigaction: libc::sigaction
}

impl SigAction {
    /// This function will set or unset the flag `SA_SIGINFO` depending on the
    /// type of the `handler` argument.
    pub fn new(handler: SigHandler, flags: SaFlags, mask: SigSet) -> SigAction {
        let mut s = unsafe { mem::uninitialized::<libc::sigaction>() };
        s.sa_sigaction = match handler {
            SigHandler::SigDfl => libc::SIG_DFL,
            SigHandler::SigIgn => libc::SIG_IGN ,
            SigHandler::Handler(f) => f as libc::sighandler_t,
            SigHandler::SigAction(f) => f as libc::sighandler_t,
        };
        s.sa_flags = match handler {
            SigHandler::SigAction(_) => (flags | SA_SIGINFO).bits(),
            _ => (flags - SA_SIGINFO).bits(),
        };
        s.sa_mask = mask.sigset;

        SigAction { sigaction: s }
    }
}

pub unsafe fn sigaction(signum: SigNum, sigaction: &SigAction) -> Result<SigAction> {
    let mut oldact = mem::uninitialized::<libc::sigaction>();

    let res =
        libc::sigaction(signum, &sigaction.sigaction as *const libc::sigaction, &mut oldact as *mut libc::sigaction);

    Errno::result(res).map(|_| SigAction { sigaction: oldact })
}

/// Manages the signal mask (set of blocked signals) for the calling thread.
///
/// If the `set` parameter is `Some(..)`, then the signal mask will be updated with the signal set.
/// The `how` flag decides the type of update. If `set` is `None`, `how` will be ignored,
/// and no modification will take place.
///
/// If the 'oldset' parameter is `Some(..)` then the current signal mask will be written into it.
///
/// If both `set` and `oldset` is `Some(..)`, the current signal mask will be written into oldset,
/// and then it will be updated with `set`.
///
/// If both `set` and `oldset` is None, this function is a no-op.
///
/// For more information, visit the [pthread_sigmask](http://man7.org/linux/man-pages/man3/pthread_sigmask.3.html),
/// or [sigprocmask](http://man7.org/linux/man-pages/man2/sigprocmask.2.html) man pages.
pub fn pthread_sigmask(how: SigFlags,
                       set: Option<&SigSet>,
                       oldset: Option<&mut SigSet>) -> Result<()> {
    if set.is_none() && oldset.is_none() {
        return Ok(())
    }

    let res = unsafe {
        // if set or oldset is None, pass in null pointers instead
        libc::pthread_sigmask(how.bits(),
                             set.map_or_else(|| ptr::null::<libc::sigset_t>(),
                                             |s| &s.sigset as *const libc::sigset_t),
                             oldset.map_or_else(|| ptr::null_mut::<libc::sigset_t>(),
                                                |os| &mut os.sigset as *mut libc::sigset_t))
    };

    Errno::result(res).map(drop)
}

pub fn kill(pid: libc::pid_t, signum: SigNum) -> Result<()> {
    let res = unsafe { libc::kill(pid, signum) };

    Errno::result(res).map(drop)
}

pub fn raise(signum: SigNum) -> Result<()> {
    let res = unsafe { libc::raise(signum) };

    Errno::result(res).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains() {
        let mut mask = SigSet::empty();
        mask.add(SIGUSR1).unwrap();

        assert_eq!(mask.contains(SIGUSR1), Ok(true));
        assert_eq!(mask.contains(SIGUSR2), Ok(false));

        let all = SigSet::all();
        assert_eq!(all.contains(SIGUSR1), Ok(true));
        assert_eq!(all.contains(SIGUSR2), Ok(true));
    }

    #[test]
    fn test_clear() {
        let mut set = SigSet::all();
        set.clear().unwrap();
        for i in 1..NSIG {
            assert_eq!(set.contains(i), Ok(false));
        }
    }

    #[test]
    fn test_extend() {
        let mut one_signal = SigSet::empty();
        one_signal.add(SIGUSR1).unwrap();

        let mut two_signals = SigSet::empty();
        two_signals.add(SIGUSR2).unwrap();
        two_signals.extend(&one_signal).unwrap();

        assert_eq!(two_signals.contains(SIGUSR1), Ok(true));
        assert_eq!(two_signals.contains(SIGUSR2), Ok(true));
    }

    #[test]
    fn test_thread_signal_block() {
        let mut mask = SigSet::empty();
        mask.add(SIGUSR1).unwrap();

        assert!(mask.thread_block().is_ok());
    }

    #[test]
    fn test_thread_signal_swap() {
        let mut mask = SigSet::empty();
        mask.add(SIGUSR1).unwrap();
        mask.thread_block().unwrap();

        assert!(SigSet::thread_get_mask().unwrap().contains(SIGUSR1).unwrap());

        let mask2 = SigSet::empty();
        mask.add(SIGUSR2).unwrap();

        let oldmask = mask2.thread_swap_mask(SIG_SETMASK).unwrap();

        assert!(oldmask.contains(SIGUSR1).unwrap());
        assert!(!oldmask.contains(SIGUSR2).unwrap());
    }

    // TODO(#251): Re-enable after figuring out flakiness.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    #[test]
    fn test_sigwait() {
        let mut mask = SigSet::empty();
        mask.add(SIGUSR1).unwrap();
        mask.add(SIGUSR2).unwrap();
        mask.thread_block().unwrap();

        raise(SIGUSR1).unwrap();
        assert_eq!(mask.wait().unwrap(), SIGUSR1);
    }
}
