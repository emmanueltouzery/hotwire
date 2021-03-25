#[macro_use]
extern crate lazy_static;

use crate::tshark_communication::TSharkCommunication;
use relm::Widget;
use std::sync::mpsc;
use std::thread;

pub mod icons;
mod tshark_communication;
mod tshark_communication_raw;
mod widgets;

// we do slow operations in a separate thread not to block the GUI thread.
// i considered that spawning a new thread everytime the GUI wants slow operations
// seems more heavyweight than reusing a thread

// https://stackoverflow.com/a/49122850/516188
pub struct BgFunc(Box<dyn Fn() + Send + 'static>);

impl BgFunc {
    pub fn new<T>(func: T) -> BgFunc
    where
        T: Fn() + Send + 'static,
    {
        BgFunc(Box::new(func))
    }
}

fn main() {
    let res_bytes = include_bytes!("icons.bin");
    let data = glib::Bytes::from(&res_bytes[..]);
    let resource = gio::Resource::from_data(&data).unwrap();
    gio::resources_register(&resource);

    let (tx, rx) = mpsc::channel::<BgFunc>();
    thread::spawn(move || {
        rx.into_iter().for_each(|fun| (fun.0)());
    });

    widgets::win::Win::run(tx).unwrap();
}
