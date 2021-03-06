use crate::TSharkPacket;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg)]
pub enum Msg {
    Quit,
}

pub struct Model {
    tree_store: gtk::TreeStore,
    streams: Vec<(Option<u32>, Vec<TSharkPacket>)>,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        let col1 = gtk::TreeViewColumnBuilder::new().title("source IP").build();
        let cell_r_txt = gtk::CellRendererText::new();
        col1.pack_start(&cell_r_txt, true);
        col1.add_attribute(&cell_r_txt, "text", 0);
        self.widgets.tree.append_column(&col1);

        // let col2 = gtk::TreeViewColumnBuilder::new()
        //     .title("source port")
        //     .build();
        // col2.pack_start(&cell_r_txt, true);
        // col2.add_attribute(&cell_r_txt, "text", 1);
        // self.widgets.tree.append_column(&col2);

        // let col3 = gtk::TreeViewColumnBuilder::new().title("dest IP").build();
        // col3.pack_start(&cell_r_txt, true);
        // col3.add_attribute(&cell_r_txt, "text", 2);
        // self.widgets.tree.append_column(&col3);

        // let col4 = gtk::TreeViewColumnBuilder::new().title("dest port").build();
        // col4.pack_start(&cell_r_txt, true);
        // col4.add_attribute(&cell_r_txt, "text", 3);
        // self.widgets.tree.append_column(&col4);

        // let col5 = gtk::TreeViewColumnBuilder::new()
        //     .title("packet count")
        //     .build();
        // col5.pack_start(&cell_r_txt, true);
        // col5.add_attribute(&cell_r_txt, "text", 4);
        // self.widgets.tree.append_column(&col5);

        self.refresh_store();
    }

    fn model(relm: &relm::Relm<Self>, streams: Vec<(Option<u32>, Vec<TSharkPacket>)>) -> Model {
        let tree_store = gtk::TreeStore::new(&[
            String::static_type(),
            // i32::static_type(),
            // String::static_type(),
            // i32::static_type(),
            // i32::static_type(),
            // String::static_type(),
        ]);
        Model {
            tree_store,
            streams,
        }
    }

    fn update(&mut self, event: Msg) {}

    fn refresh_store(&mut self) {
        self.model.tree_store.clear();
        for stream in &self.model.streams {
            let iter = self.model.tree_store.append(None);
            let layers = &stream.1.first().unwrap().source.layers;
            self.model.tree_store.set_value(
                &iter,
                0,
                &format!(
                    "{}:{} -> {}:{}",
                    layers.ip.as_ref().unwrap().ip_src,
                    layers.tcp.as_ref().unwrap().port_src,
                    layers.ip.as_ref().unwrap().ip_dst,
                    layers.tcp.as_ref().unwrap().port_dst
                )
                .to_value(),
            );
            // self.model.tree_store.set_value(
            //     &iter,
            //     1,
            //     &layers.tcp.as_ref().unwrap().port_src.to_value(),
            // );
            // self.model.tree_store.set_value(
            //     &iter,
            //     2,
            //     &layers.ip.as_ref().unwrap().ip_dst.to_value(),
            // );
            // self.model.tree_store.set_value(
            //     &iter,
            //     3,
            //     &layers.tcp.as_ref().unwrap().port_dst.to_value(),
            // );
            // self.model
            //     .tree_store
            //     .set_value(&iter, 4, &(stream.1.len() as i64).to_value());
            for request in &stream.1 {
                // search for the field which is an object and for which the object contains a field "http.request.method"
                let child = self.model.tree_store.append(Some(&iter));
                if let Some(serde_json::Value::Object(http_map)) =
                    request.source.layers.http.as_ref()
                {
                    let missing = format!("{:?}", http_map);
                    let req_info = http_map.iter().find(|(_,v)| matches!(v,
                        serde_json::Value::Object(fields) if fields.contains_key("http.request.method") || fields.contains_key("http.response.code")
                    )).unwrap_or((&missing, &serde_json::json!(null))).0;
                    self.model
                        .tree_store
                        .set_value(&child, 0, &req_info.to_value());
                }
            }
        }
    }

    view! {
        gtk::Window {
            gtk::ScrolledWindow {
                #[name="tree"]
                gtk::TreeView {
                    model: Some(&self.model.tree_store)
                }
            }
        }
    }
}
