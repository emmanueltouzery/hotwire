use super::http_comm_entry::{HttpCommEntry, HttpMessageData};
use gtk::prelude::*;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::collections::HashMap;

#[derive(Msg)]
pub enum Msg {}

pub struct HttpCommRemoteServerData {
    pub remote_ip: String,
    pub tcp_sessions: Vec<(Option<u32>, Vec<HttpMessageData>)>,
}

pub struct Model {
    data: HttpCommRemoteServerData,
}

#[widget]
impl Widget for HttpCommRemoteServer {
    fn init_view(&mut self) {
        self.refresh_comm_entries();
    }

    fn model(relm: &relm::Relm<Self>, data: HttpCommRemoteServerData) -> Model {
        Model { data }
    }

    fn update(&mut self, event: Msg) {}

    fn refresh_comm_entries(&self) {
        let mut comm_entries_group_start_indexes = HashMap::new();
        let mut row_idx = 0;
        for tcp_session in &self.model.data.tcp_sessions {
            comm_entries_group_start_indexes
                .insert(row_idx, format!("tcp session {:?}", tcp_session.0));
            for msg in &tcp_session.1 {
                self.widgets
                    .http_comm_entries
                    .add_widget::<HttpCommEntry>((*msg).clone());
                row_idx += 1;
            }
        }
        self.widgets
            .http_comm_entries
            .set_header_func(Some(Box::new(move |row, _h| {
                if let Some(group_name) =
                    comm_entries_group_start_indexes.get(&(row.get_index() as usize))
                {
                    let vbox = gtk::BoxBuilder::new()
                        .orientation(gtk::Orientation::Vertical)
                        .build();
                    vbox.add(&gtk::SeparatorBuilder::new().build());
                    let label = gtk::LabelBuilder::new()
                        .label(&format!("<b>{}</b>", group_name))
                        .use_markup(true)
                        .xalign(0.0)
                        .build();
                    label.get_style_context().add_class("project_item_header");
                    vbox.add(&label);
                    vbox.show_all();
                    row.set_header(Some(&vbox));
                } else {
                    row.set_header::<gtk::ListBoxRow>(None)
                }
            })));
    }

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            gtk::Label {
                label: &self.model.data.remote_ip
            },
            #[name="http_comm_entries"]
            gtk::ListBox {
            },
        }
    }
}
