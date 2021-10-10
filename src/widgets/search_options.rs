use crate::search_expr::SearchOperator;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::collections::HashSet;

#[derive(Copy, Clone)]
pub enum CombineOperator {
    And,
    Or,
}

#[derive(Msg)]
pub enum Msg {
    FilterKeysUpdated(HashSet<&'static str>),
    AddClick,
    AddAndCloseClick,
    Add(
        (
            Option<CombineOperator>,
            &'static str,
            SearchOperator,
            String,
        ),
    ),
    DisableOptions,
    EnableOptionsWithAndOr,
    EnableOptionsWithoutAndOr,
}

pub struct Model {
    relm: relm::Relm<SearchOptions>,
    filter_keys: HashSet<&'static str>,
}

#[widget]
impl Widget for SearchOptions {
    fn init_view(&mut self) {
        self.widgets.and_or_combo.append_text("and");
        self.widgets.and_or_combo.append_text("or");
        self.widgets.and_or_combo.set_active(Some(0));
        self.widgets.filter_key_combo.append_text("grid.cells");
        self.widgets.filter_key_combo.append_text("detail.contents");
        self.widgets.filter_key_combo.set_active(Some(0));
        self.widgets.search_op_combo.append_text("contains");
        self.widgets.search_op_combo.set_active(Some(0));
    }

    fn model(relm: &relm::Relm<Self>, filter_keys: HashSet<&'static str>) -> Model {
        Model {
            relm: relm.clone(),
            filter_keys,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::FilterKeysUpdated(keys) => {
                self.model.filter_keys = keys;
            }
            Msg::DisableOptions => {
                self.widgets.root_grid.set_sensitive(false);
            }
            Msg::EnableOptionsWithAndOr => {
                self.widgets.root_grid.set_sensitive(true);
                self.widgets.and_or_combo.set_sensitive(true);
            }
            Msg::EnableOptionsWithoutAndOr => {
                self.widgets.root_grid.set_sensitive(true);
                self.widgets.and_or_combo.set_sensitive(false);
            }
            Msg::AddClick => {
                self.add_clicked();
            }
            Msg::AddAndCloseClick => {
                self.add_clicked();
            }
            // meant for my parent
            Msg::Add(_) => {}
        }
    }

    fn add_clicked(&mut self) {
        let combine_operator = if self.widgets.and_or_combo.is_sensitive() {
            Some(
                if self.widgets.and_or_combo.active_text() == Some("and".into()) {
                    CombineOperator::And
                } else {
                    CombineOperator::Or
                },
            )
        } else {
            None
        };
        let filter_key = self
            .widgets
            .filter_key_combo
            .active_text()
            .as_ref()
            .and_then(|fk| self.model.filter_keys.get(fk.as_str()))
            .unwrap();
        let search_op = match self
            .widgets
            .search_op_combo
            .active_text()
            .as_ref()
            .map(|k| k.as_str())
        {
            Some("contains") => SearchOperator::Contains,
            x => panic!("unhandled search_op: {:?}", x),
        };
        let search_txt = self.widgets.search_entry.text().to_string();
        self.model.relm.stream().emit(Msg::Add((
            combine_operator,
            filter_key,
            search_op,
            search_txt,
        )));
    }

    view! {
        #[name="root_grid"]
         gtk::Grid {
             orientation: gtk::Orientation::Vertical,
             margin_top: 10,
             margin_start: 10,
             margin_end: 10,
             margin_bottom: 10,
             row_spacing: 5,
             column_spacing: 10,
             #[name="and_or_combo"]
             gtk::ComboBoxText {
                 cell: {
                     left_attach: 0,
                     top_attach: 0,
                 },
             },
             #[name="filter_key_combo"]
             gtk::ComboBoxText {
                 cell: {
                     left_attach: 1,
                     top_attach: 0,
                 },
             },
             #[name="search_op_combo"]
             gtk::ComboBoxText {
                 cell: {
                     left_attach: 0,
                     top_attach: 1,
                 },
             },
             #[name="search_entry"]
             gtk::SearchEntry {
                 cell: {
                     left_attach: 1,
                     top_attach: 1,
                 },
             },
             gtk::ButtonBox {
                 cell: {
                     left_attach: 0,
                     top_attach: 2,
                     width: 2,
                 },
                 layout_style: gtk::ButtonBoxStyle::Expand,
                 gtk::Button {
                     label: "Add",
                     clicked => Msg::AddClick,
                 },
                 gtk::Button {
                     label: "Add and close",
                     clicked => Msg::AddAndCloseClick,
                 },
             },
         },
    }
}
