use crate::widgets::headerbar_search;
use relm::Widget;
#[cfg(target_family = "unix")]
use std::os::unix::fs::FileTypeExt;
use std::sync::mpsc;
use std::thread;
use widgets::win;

pub mod colors;
pub mod config;
pub mod icons;
pub mod message_parser;
pub mod message_parsers;
pub mod packets_read;
pub mod search_expr;
pub mod streams;
#[macro_use]
mod tshark_communication;
mod widgets;

pub mod http;
pub mod http2;
pub mod pgsql;

#[macro_use]
extern crate lazy_static;

// we do slow operations in a separate thread not to block the GUI thread.
// i considered that spawning a new thread everytime the GUI wants slow operations
// seems more heavyweight than reusing a thread

// https://stackoverflow.com/a/49122850/516188
pub struct BgFunc(Box<dyn FnMut() + Send + 'static>);

impl BgFunc {
    pub fn new<T>(func: T) -> BgFunc
    where
        T: FnMut() + Send + 'static,
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
        rx.into_iter().for_each(|mut fun| (fun.0)());
    });
    let mut args = std::env::args();
    args.next();

    if let Err(e) = config::remove_obsolete_tcpdump_files(config::RemoveMode::OldFilesOnly) {
        eprintln!("Error removing obsolete tcpdump files: {}", e);
    }

    let path = args.next().map(|param_p| {
        let p = tshark_communication::string_to_path(&param_p);
        let is_fifo = if cfg!(unix) {
            std::fs::metadata(&p)
                .ok()
                .map(|m| m.file_type().is_fifo())
                .is_some()
        } else {
            false
        };
        (
            p,
            if is_fifo {
                packets_read::TSharkInputType::Fifo
            } else {
                packets_read::TSharkInputType::File
            },
        )
    });

    let recent_searches = match headerbar_search::HeaderbarSearch::load_recent_list() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error loading recent files: {}", e);
            vec![]
        }
    };

    win::Win::run((tx, path, recent_searches)).unwrap();
}
