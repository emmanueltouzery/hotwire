pub struct Tls;
use crate::icons::Icon;
use crate::widgets::comm_remote_server::{MessageData, MessageParser, MessageParserDetailsMsg};
use crate::BgFunc;
use crate::TSharkCommunication;
use gtk::prelude::*;
use relm::{ContainerWidget, Widget};
use relm_derive::widget;
use std::sync::mpsc;

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct TlsMessageData {}

impl MessageParser for Tls {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.tls.is_some()
    }

    fn protocol_icon(&self) -> Icon {
        Icon::LOCK
    }

    fn parse_stream(&self, stream: &[TSharkCommunication]) -> Vec<MessageData> {
        vec![MessageData::Tls(TlsMessageData {})]
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) -> gtk::ListStore {
        let liststore = gtk::ListStore::new(&[
            String::static_type(), // description
            i32::static_type(), // dummy (win has list store columns 2 & 3 hardcoded for stream & row idx)
            u32::static_type(), // stream_id
            u32::static_type(), // index of the comm in the model vector
        ]);

        let data_col = gtk::TreeViewColumnBuilder::new()
            .title("TLS")
            .expand(true)
            .resizable(true)
            .build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new()
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        data_col.pack_start(&cell_r_txt, true);
        data_col.add_attribute(&cell_r_txt, "text", 0);
        tv.append_column(&data_col);
        tv.set_model(Some(&liststore));

        liststore
    }

    fn populate_treeview(&self, ls: &gtk::ListStore, session_id: u32, messages: &Vec<MessageData>) {
        let iter = ls.append();
        ls.set_value(&iter, 0, &"Encrypted TLS stream".to_value());
        ls.set_value(&iter, 2, &session_id.to_value());
        ls.set_value(&iter, 3, &0.to_value());
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        bg_sender: mpsc::Sender<BgFunc>,
    ) -> relm::StreamHandle<MessageParserDetailsMsg> {
        let component = Box::leak(Box::new(
            parent.add_widget::<TlsCommEntry>(TlsMessageData {}),
        ));
        component.stream()
    }
}

pub struct Model {}

#[widget]
impl Widget for TlsCommEntry {
    fn model(relm: &relm::Relm<Self>, data: TlsMessageData) -> Model {
        Model {}
    }

    fn update(&mut self, event: MessageParserDetailsMsg) {}

    view! {
        gtk::Box {
            gtk::Label {
                label: "The contents of this stream are encrypted."
            }
        }
    }
}
