use crate::tshark_communication::TSharkCommunication;
use relm::Widget;

pub mod icons;
mod tshark_communication;
mod widgets;

fn main() {
    let res_bytes = include_bytes!("icons.bin");
    let data = glib::Bytes::from(&res_bytes[..]);
    let resource = gio::Resource::from_data(&data).unwrap();
    gio::resources_register(&resource);

    widgets::win::Win::run(()).unwrap();
}
