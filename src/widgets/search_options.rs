use crate::search_expr::{OperatorNegation, SearchOperator};
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::collections::BTreeSet;

const INVALID_SEARCH_STACK_NAME: &str = "invalid-search";
const VALID_SEARCH_STACK_NAME: &str = "valid-search";

#[derive(Copy, Clone)]
pub enum CombineOperator {
    And,
    Or,
}

#[derive(Msg)]
pub enum Msg {
    ParentSet(gtk::Popover),
    FilterKeysUpdated(BTreeSet<&'static str>),
    SearchEntryKeyPress(gdk::EventKey),
    AddClick,
    AddAndCloseClick,
    ClearSearchTextClick,
    Add(
        (
            Option<CombineOperator>,
            &'static str,
            SearchOperator,
            OperatorNegation,
            String,
        ),
    ),
    DisableOptions,
    EnableOptionsWithAndOr,
    EnableOptionsWithoutAndOr,
}

pub struct Model {
    relm: relm::Relm<SearchOptions>,
    filter_keys: BTreeSet<&'static str>,
}

#[widget]
impl Widget for SearchOptions {
    fn init_view(&mut self) {
        self.widgets.and_or_combo.append_text("and");
        self.widgets.and_or_combo.append_text("or");
        self.widgets.and_or_combo.set_active(Some(0));
        for k in self.model.filter_keys.iter() {
            self.widgets.filter_key_combo.append_text(k);
        }
        self.widgets.filter_key_combo.set_active(Some(0));
        self.widgets.search_op_combo.append_text("contains");
        self.widgets.search_op_combo.append_text("doesntContain");
        self.widgets.search_op_combo.set_active(Some(0));
    }

    fn model(relm: &relm::Relm<Self>, filter_keys: BTreeSet<&'static str>) -> Model {
        Model {
            relm: relm.clone(),
            filter_keys,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::ParentSet(popover) => {
                self.widgets.add_and_close_btn.set_can_default(true);
                popover.set_default_widget(Some(&self.widgets.add_and_close_btn));
            }
            Msg::FilterKeysUpdated(keys) => {
                self.model.filter_keys = keys;
                self.widgets.filter_key_combo.remove_all();
                for k in self.model.filter_keys.iter() {
                    self.widgets.filter_key_combo.append_text(k);
                }
                self.widgets.filter_key_combo.set_active(Some(0));
            }
            Msg::SearchEntryKeyPress(e) => {
                if e.state().contains(gdk::ModifierType::CONTROL_MASK)
                    && (e.keyval() == gdk::keys::constants::Return
                        || e.keyval() == gdk::keys::constants::KP_Enter)
                {
                    self.model.relm.stream().emit(Msg::AddClick);
                }
            }
            Msg::DisableOptions => {
                self.widgets
                    .root_stack
                    .set_visible_child_name(INVALID_SEARCH_STACK_NAME);
            }
            Msg::EnableOptionsWithAndOr => {
                self.widgets
                    .root_stack
                    .set_visible_child_name(VALID_SEARCH_STACK_NAME);
                self.widgets.and_or_combo.set_sensitive(true);
            }
            Msg::EnableOptionsWithoutAndOr => {
                self.widgets
                    .root_stack
                    .set_visible_child_name(VALID_SEARCH_STACK_NAME);
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
            Msg::ClearSearchTextClick => {}
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
        let (search_op, op_negation) = match self
            .widgets
            .search_op_combo
            .active_text()
            .as_ref()
            .map(|k| k.as_str())
        {
            Some("contains") => (SearchOperator::Contains, OperatorNegation::NotNegated),
            Some("doesntContain") => (SearchOperator::Contains, OperatorNegation::Negated),
            x => panic!("unhandled search_op: {:?}", x),
        };
        let search_txt = self.widgets.search_entry.text().to_string();
        self.model.relm.stream().emit(Msg::Add((
            combine_operator,
            filter_key,
            search_op,
            op_negation,
            search_txt,
        )));
    }

    view! {
        #[name="root_stack"]
        gtk::Stack {
            gtk::Grid {
                child: {
                    name: Some(VALID_SEARCH_STACK_NAME)
                },
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
                    key_press_event(_, event) => (Msg::SearchEntryKeyPress(event.clone()), Inhibit(false)),
                    activates_default: true,
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
                    #[name="add_and_close_btn"]
                    gtk::Button {
                        label: "Add and close",
                        can_default: true,
                        has_default: true,
                        clicked => Msg::AddAndCloseClick,
                    },
                },
            },
            gtk::Box {
                orientation: gtk::Orientation::Vertical,
                margin_top: 20,
                margin_start: 10,
                margin_end: 10,
                margin_bottom: 10,
                spacing: 10,
                child: {
                    name: Some(INVALID_SEARCH_STACK_NAME)
                },
                gtk::Label {
                    text: "Failed to parse the search string. Please correct the search string.",
                    max_width_chars: 30,
                    line_wrap: true,
                },
                #[style_class="destructive-action"]
                gtk::Button {
                    child: {
                        pack_type: gtk::PackType::End,
                    },
                    label: "Clear search text",
                    clicked => Msg::ClearSearchTextClick,
                }
            }
        },
    }
}
