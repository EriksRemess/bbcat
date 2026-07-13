#[cfg(target_os = "linux")]
mod linux {
    use std::{
        ffi::c_int,
        fs::OpenOptions,
        io::{self, Read, Write},
        mem::MaybeUninit,
        os::fd::{AsRawFd, RawFd},
        time::{Duration, Instant},
    };

    const QUERY: &[u8] = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c";
    const TIMEOUT: Duration = Duration::from_millis(750);
    const TCSANOW: c_int = 0;
    const ICANON: u32 = 0x0000_0002;
    const ECHO: u32 = 0x0000_0008;
    const VTIME: usize = 5;
    const VMIN: usize = 6;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Termios {
        input_flags: u32,
        output_flags: u32,
        control_flags: u32,
        local_flags: u32,
        line_discipline: u8,
        control_characters: [u8; 32],
        input_speed: u32,
        output_speed: u32,
    }

    unsafe extern "C" {
        fn tcgetattr(fd: c_int, termios: *mut Termios) -> c_int;
        fn tcsetattr(fd: c_int, action: c_int, termios: *const Termios) -> c_int;
    }

    struct RestoreTerminal {
        fd: RawFd,
        original: Termios,
    }

    impl Drop for RestoreTerminal {
        fn drop(&mut self) {
            // Best effort: there is no useful recovery if restoring the TTY fails.
            unsafe {
                tcsetattr(self.fd, TCSANOW, &raw const self.original);
            }
        }
    }

    pub fn supports_kitty() -> Result<bool, String> {
        let mut tty = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .map_err(|error| format!("cannot open controlling terminal: {error}"))?;
        let fd = tty.as_raw_fd();
        let original = get_termios(fd)
            .map_err(|error| format!("cannot inspect controlling terminal: {error}"))?;
        let _restore = RestoreTerminal { fd, original };

        let mut probe_mode = original;
        probe_mode.local_flags &= !(ICANON | ECHO);
        probe_mode.control_characters[VMIN] = 0;
        probe_mode.control_characters[VTIME] = 1;
        set_termios(fd, &probe_mode)
            .map_err(|error| format!("cannot prepare terminal protocol probe: {error}"))?;

        tty.write_all(QUERY)
            .and_then(|()| tty.flush())
            .map_err(|error| format!("cannot write terminal protocol probe: {error}"))?;

        let deadline = Instant::now() + TIMEOUT;
        let mut response = Vec::with_capacity(128);
        let mut buffer = [0_u8; 128];
        while Instant::now() < deadline {
            match tty.read(&mut buffer) {
                Ok(0) => {}
                Ok(length) => {
                    response.extend_from_slice(&buffer[..length]);
                    if let Some(supported) = probe_result(&response) {
                        return Ok(supported);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("cannot read terminal protocol reply: {error}")),
            }
        }

        Ok(false)
    }

    fn get_termios(fd: RawFd) -> io::Result<Termios> {
        let mut value = MaybeUninit::uninit();
        if unsafe { tcgetattr(fd, value.as_mut_ptr()) } == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { value.assume_init() })
        }
    }

    fn set_termios(fd: RawFd, value: &Termios) -> io::Result<()> {
        if unsafe { tcsetattr(fd, TCSANOW, value) } == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn probe_result(response: &[u8]) -> Option<bool> {
        has_device_attributes(response).then(|| contains(response, b"\x1b_Gi=31;"))
    }

    fn has_device_attributes(response: &[u8]) -> bool {
        response.windows(2).enumerate().any(|(index, prefix)| {
            prefix == b"\x1b["
                && response[index + 2..]
                    .iter()
                    .take(64)
                    .any(|&byte| byte == b'c')
        })
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn recognizes_a_supported_terminal_after_the_da_marker() {
            let response = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c";
            assert_eq!(probe_result(response), Some(true));
        }

        #[test]
        fn recognizes_an_unsupported_terminal_after_the_da_marker() {
            assert_eq!(probe_result(b"\x1b[?1;2c"), Some(false));
        }

        #[test]
        fn waits_for_the_device_attributes_marker() {
            assert_eq!(probe_result(b"\x1b_Gi=31;OK\x1b\\"), None);
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::supports_kitty;

#[cfg(not(target_os = "linux"))]
pub fn supports_kitty() -> Result<bool, String> {
    Ok(false)
}
