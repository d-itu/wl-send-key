use core::{mem::MaybeUninit, num};
use std::process::ExitCode;

use rustix::{
    fd::AsFd,
    io::{IoSlice, IoSliceMut},
    net::{
        AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
        SendAncillaryMessage, SendFlags, SocketAddrUnix, SocketType,
    },
};
use tuplez::tuple;
use wayland_oxide::{
    Buffer, Complete, Fd, GenericNewId, Interface, MessageEnum as _, Object,
    buffer::{InvalidMessageSize, Message},
    unmarshal,
};

use crate::wl_keyboard::enums::KeyState;

include!(concat!(env!("OUT_DIR"), "/wayland.rs"));

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("io error")]
    Io(
        #[source]
        #[from]
        rustix::io::Errno,
    ),
    #[error(transparent)]
    Unmarshal(#[from] unmarshal::Error),
    #[error(transparent)]
    InvalidMessageSize(#[from] InvalidMessageSize),
    #[error("no fd")]
    NoFdReceived,
    #[error("cannot parse arg")]
    Arg(
        #[source]
        #[from]
        num::ParseIntError,
    ),
    #[error("missing args")]
    MissingArgs,
    #[error("{} not supported", _0.name)]
    NotSupported(Interface),
}

struct Allocator {
    acc: u32,
}

impl Allocator {
    fn new() -> Self {
        Self { acc: 1 }
    }
    fn next<P: From<Object>>(&mut self) -> P {
        let res = P::from(unsafe { Object::from_u32(self.acc) });
        self.acc += 1;
        res
    }
}

struct Connection {
    socket: rustix::fd::OwnedFd,
}

impl Connection {
    fn send(&mut self, msg: impl Complete) -> rustix::io::Result<usize> {
        rustix::io::write(&self.socket, &msg.marshal_to_boxed_slice())
    }
    fn receive(&mut self, buffer: &mut Buffer) -> rustix::io::Result<()> {
        unsafe {
            let (read, _) = rustix::io::read(&self.socket, buffer.unfilled())?;
            let n = read.len();
            buffer.advance(n);
        }
        Ok(())
    }
    fn send_with_fd(&mut self, msg: impl Complete, fd: impl AsFd) -> rustix::io::Result<usize> {
        let mut ancillary_buffer = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut send_ancillary_buffer = SendAncillaryBuffer::new(&mut ancillary_buffer);
        let fds = [fd.as_fd()];
        let ok = send_ancillary_buffer.push(SendAncillaryMessage::ScmRights(fds.as_slice()));
        debug_assert!(ok);
        rustix::net::sendmsg(
            self.socket.as_fd(),
            &[IoSlice::new(&msg.marshal_to_boxed_slice())],
            &mut send_ancillary_buffer,
            SendFlags::empty(),
        )
    }
}

fn run() -> Result<(), Error> {
    let socket = rustix::net::socket(AddressFamily::UNIX, SocketType::STREAM, None)?;
    rustix::net::connect(
        &socket,
        &SocketAddrUnix::new(
            wayland_oxide::socket_path_from_env()
                .as_os_str()
                .as_encoded_bytes(),
        )?,
    )?;
    let mut connection = Connection { socket };

    let mut allocator = Allocator::new();
    let mut buffer = Buffer::new();

    let display: WlDisplay = allocator.next();
    let registry = allocator.next();
    let roundtrip = allocator.next();

    connection.send(tuple!(
        display.get_registry(registry),
        display.sync(roundtrip),
    ))?;

    let mut virtual_keyboard_manager: Option<ZwpVirtualKeyboardManagerV1> = None;
    let mut seat: Option<WlSeat> = None;

    'out: loop {
        connection.receive(&mut buffer)?;
        for item in buffer.drain() {
            let Message {
                object,
                opcode,
                reader,
            } = item?;
            if object == registry.id().as_u32() {
                match wl_registry::Event::new(opcode, reader) {
                    wl_registry::Event::Global(lazy) => {
                        if let Ok(wl_registry::Global {
                            name,
                            interface,
                            version,
                        }) = lazy.parse()
                        {
                            match interface.as_bytes() {
                                b"zwp_virtual_keyboard_manager_v1" => {
                                    let id = virtual_keyboard_manager.insert(allocator.next()).id();
                                    connection.send(registry.bind(
                                        name,
                                        GenericNewId {
                                            interface: "zwp_virtual_keyboard_manager_v1",
                                            version,
                                            id,
                                        },
                                    ))?;
                                }
                                b"wl_seat" => {
                                    let id = seat.insert(allocator.next()).id();
                                    connection.send(registry.bind(
                                        name,
                                        GenericNewId {
                                            interface: "wl_seat",
                                            version,
                                            id,
                                        },
                                    ))?;
                                }
                                _ => {}
                            }
                        }
                    }
                    wl_registry::Event::GlobalRemove(_) => {}
                    wl_registry::Event::_Unknown(_) => {}
                }
                continue;
            }
            if object == roundtrip.id().as_u32() {
                break 'out;
            }
        }
    }

    let seat = seat.ok_or(Error::NotSupported(wl_seat::INTERFACE))?;
    let virtual_keyboard_manager = virtual_keyboard_manager.ok_or(Error::NotSupported(
        zwp_virtual_keyboard_manager_v1::INTERFACE,
    ))?;

    let keyboard = allocator.next();
    let virtual_keyboard = allocator.next();
    connection.send(tuple!(
        seat.get_keyboard(keyboard),
        virtual_keyboard_manager.create_virtual_keyboard(seat, virtual_keyboard),
    ))?;

    'out: loop {
        let io_slice = IoSliceMut::new(unsafe { buffer.unfilled().assume_init_mut() });
        let mut control_buffer = Buffer::new();
        let mut control = RecvAncillaryBuffer::new(control_buffer.unfilled());
        let n = rustix::net::recvmsg(
            &connection.socket,
            &mut [io_slice],
            &mut control,
            RecvFlags::empty(),
        )?
        .bytes;
        assert!(n < buffer.unfilled().len());
        unsafe { buffer.advance(n) };
        let mut fds = control
            .drain()
            .filter_map(|x| {
                if let RecvAncillaryMessage::ScmRights(x) = x {
                    Some(x)
                } else {
                    None
                }
            })
            .flatten();
        for item in buffer.drain() {
            let Message {
                object,
                opcode,
                reader,
            } = item?;
            if object == keyboard.id().as_u32()
                && let wl_keyboard::Event::Keymap(lazy) = wl_keyboard::Event::new(opcode, reader)
            {
                let wl_keyboard::Keymap { format, size, .. } = lazy.parse()?;
                let fd = fds.next().ok_or(Error::NoFdReceived)?;
                connection.send_with_fd(virtual_keyboard.keymap(format.as_u32(), Fd, size), fd)?;
                break 'out;
            }
        }
    }

    let mut args = std::env::args().skip(1);
    let modifier = args.next().ok_or(Error::MissingArgs)?.parse()?;
    let key = args.next().ok_or(Error::MissingArgs)?.parse()?;

    let roundtrip = allocator.next();
    connection.send(tuple!(
        virtual_keyboard.modifiers(modifier, 0, 0, 0),
        virtual_keyboard.key(0, key, KeyState::Pressed as u32),
        virtual_keyboard.key(0, key, KeyState::Released as u32),
        display.sync(roundtrip),
    ))?;
    'out: loop {
        connection.receive(&mut buffer)?;
        for item in buffer.drain() {
            let Message {
                object,
                opcode,
                reader,
            } = item?;
            if object == display.id().as_u32() {
                match wl_display::Event::new(opcode, reader) {
                    wl_display::Event::Error(lazy) => {
                        dbg!(lazy.parse()?);
                        break 'out;
                    }
                    wl_display::Event::DeleteId(_) => {}
                    wl_display::Event::_Unknown(_) => {}
                }
            }
            if object == roundtrip.id().as_u32() {
                break 'out;
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
