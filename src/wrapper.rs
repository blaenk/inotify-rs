#![unstable]
#![allow(missing_docs)]

//! Idiomatic wrapper for inotify

use libc::{
    EAGAIN,
    EWOULDBLOCK,
    F_GETFL,
    F_SETFL,
    O_NONBLOCK,
    fcntl,
    c_int,
    c_void,
    size_t,
    ssize_t
};
use std::ffi::{
    CString,
};
use std::mem;
use std::io;
use std::os::errno;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ffi::AsOsStr;
use std::slice;

use ffi;
use ffi::inotify_event;


pub type Watch = c_int;

#[derive(Clone)]
pub struct INotify {
    pub fd: c_int,
    events: Vec<Event>,
}

impl INotify {
    pub fn init() -> io::Result<INotify> {
        INotify::init_with_flags(0)
    }

    pub fn init_with_flags(flags: isize) -> io::Result<INotify> {
        let fd = unsafe { ffi::inotify_init1(flags as c_int) };

        unsafe { fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK) };

        match fd {
            -1 => Err(io::Error::last_os_error()),
            _  => Ok(INotify {
                fd    : fd,
                events: Vec::new(),
            })
        }
    }

    pub fn add_watch(&self, path: &Path, mask: u32) -> io::Result<Watch> {
        let wd = unsafe {
            let c_str = try!(path.as_os_str().to_cstring());
            ffi::inotify_add_watch(
                self.fd,
                c_str.as_ptr(),
                mask
            )
        };

        match wd {
            -1 => Err(io::Error::last_os_error()),
            _  => Ok(wd)
        }
    }

    pub fn rm_watch(&self, watch: Watch) -> io::Result<()> {
        let result = unsafe { ffi::inotify_rm_watch(self.fd, watch) };
        match result {
            0  => Ok(()),
            -1 => Err(io::Error::last_os_error()),
            _  => panic!(
                "unexpected return code from inotify_rm_watch ({})", result)
        }
    }

    /// Wait until events are available, then return them.
    /// This function will block until events are available. If you want it to
    /// return immediately, use `available_events`.
    pub fn wait_for_events(&mut self) -> io::Result<&[Event]> {
        let fd = self.fd;

        unsafe {
            fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) & !O_NONBLOCK)
        };
        let result = self.available_events();
        unsafe {
            fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
        };

        result
    }

    /// Returns available inotify events.
    /// If no events are available, this method will simply return a slice with
    /// zero events. If you want to wait for events to become available, call
    /// `wait_for_events`.
    pub fn available_events(&mut self) -> io::Result<&[Event]> {
        self.events.clear();

        let mut buffer = [0u8; 1024];
        let len = unsafe {
            ffi::read(
                self.fd,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as size_t
            )
        };

        match len {
            0 => {
                return Err(
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "buffer passed to read() was too small to read event",
                        None));
            }
            -1 => {
                let error = errno();
                if error == EAGAIN as i32 || error == EWOULDBLOCK as i32 {
                    return Ok(&self.events[..]);
                }
                else {
                    return Err(io::Error::from_os_error(error));
                }
            },
            _ =>
                ()
        }

        let event_size = mem::size_of::<inotify_event>();

        let mut i = 0;
        while i < len {
            unsafe {
                let slice = &buffer[i as usize..];

                let event = slice.as_ptr() as *const inotify_event;

                let name = if (*event).len > 0 {
                    let name_ptr = slice
                        .as_ptr()
                        .offset(event_size as isize);

                    let mut name_slice = slice::from_raw_parts(
                        name_ptr,
                        (*event).len as usize,
                    );

                    // Make sure slice contains no \0, CString doesn't like
                    // them.
                    let pos = name_slice.position_elem(&0);
                    match pos {
                        Some(pos) => name_slice = &name_slice[..pos],
                        None      => (),
                    }

                    let c_str = try!(CString::new(name_slice));

                    match String::from_utf8(c_str.as_bytes().to_vec()) {
                        Ok(string)
                            => string.to_string(),
                        Err(e) =>
                            panic!("Failed to convert C string into Rust string: {}", e)
                    }
                }
                else {
                    "".to_string()
                };

                self.events.push(Event::new(&*event, name));

                i += (event_size + (*event).len as usize) as ssize_t;
            }
        }

        Ok(&self.events[..])
    }

    pub fn close(&self) -> io::Result<()> {
        let result = unsafe { ffi::close(self.fd) };
        match result {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error())
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub wd    : i32,
    pub mask  : u32,
    pub cookie: u32,
    pub name  : String,
}

impl Event {
    fn new(event: &inotify_event, name: String) -> Event {
        Event {
            wd    : event.wd,
            mask  : event.mask,
            cookie: event.cookie,
            name  : name,
        }
    }

    pub fn is_access(&self) -> bool {
        return self.mask & ffi::IN_ACCESS > 0;
    }

    pub fn is_modify(&self) -> bool {
        return self.mask & ffi::IN_MODIFY > 0;
    }

    pub fn is_attrib(&self) -> bool {
        return self.mask & ffi::IN_ATTRIB > 0;
    }

    pub fn is_close_write(&self) -> bool {
        return self.mask & ffi::IN_CLOSE_WRITE > 0;
    }

    pub fn is_close_nowrite(&self) -> bool {
        return self.mask & ffi::IN_CLOSE_NOWRITE > 0;
    }

    pub fn is_open(&self) -> bool {
        return self.mask & ffi::IN_OPEN > 0;
    }

    pub fn is_moved_from(&self) -> bool {
        return self.mask & ffi::IN_MOVED_FROM > 0;
    }

    pub fn is_moved_to(&self) -> bool {
        return self.mask & ffi::IN_MOVED_TO > 0;
    }

    pub fn is_create(&self) -> bool {
        return self.mask & ffi::IN_CREATE > 0;
    }

    pub fn is_delete(&self) -> bool {
        return self.mask & ffi::IN_DELETE > 0;
    }

    pub fn is_delete_self(&self) -> bool {
        return self.mask & ffi::IN_DELETE_SELF > 0;
    }

    pub fn is_move_self(&self) -> bool {
        return self.mask & ffi::IN_MOVE_SELF > 0;
    }

    pub fn is_move(&self) -> bool {
        return self.mask & ffi::IN_MOVE > 0;
    }

    pub fn is_close(&self) -> bool {
        return self.mask & ffi::IN_CLOSE > 0;
    }

    pub fn is_dir(&self) -> bool {
        return self.mask & ffi::IN_ISDIR > 0;
    }

    pub fn is_unmount(&self) -> bool {
        return self.mask & ffi::IN_UNMOUNT > 0;
    }

    pub fn is_queue_overflow(&self) -> bool {
        return self.mask & ffi::IN_Q_OVERFLOW > 0;
    }

    pub fn is_ignored(&self) -> bool {
        return self.mask & ffi::IN_IGNORED > 0;
    }
}
