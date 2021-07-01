use super::win;
use crate::config::Config;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg)]
pub enum Msg {
    DarkThemeToggled(bool),
    CustomTcpDumpBufferSizeToggled(bool),
    CustomTcpDumpBufferSizeValueChanged,
}

pub struct Model {
    prefer_dark_theme: bool,
    custom_tcpdump_buffer_size_kib: Option<usize>,
}

#[widget]
impl Widget for Preferences {
    fn init_view(&mut self) {
        self.widgets
            .tcpdump_buf_size_spinner
            .set_range(1024.0, 32768.0);
        let config = Config::read_config();
        // I believe the default is 2048 KiB https://serverfault.com/a/421793/176574
        self.widgets
            .tcpdump_buf_size_spinner
            .set_value(config.custom_tcpdump_buffer_size_kib.unwrap_or(2048) as f64);
        self.widgets
            .tcpdump_buf_size_spinner
            .set_increments(128.0, 128.0);
    }

    fn model() -> Model {
        let config = Config::read_config();
        Model {
            prefer_dark_theme: config.prefer_dark_theme,
            custom_tcpdump_buffer_size_kib: config.custom_tcpdump_buffer_size_kib,
        }
    }

    fn get_config(&self) -> Config {
        Config {
            prefer_dark_theme: self.model.prefer_dark_theme,
            custom_tcpdump_buffer_size_kib: self.model.custom_tcpdump_buffer_size_kib,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::DarkThemeToggled(t) => {
                gtk::Settings::get_default()
                    .unwrap()
                    .set_property_gtk_application_prefer_dark_theme(t);
                self.model.prefer_dark_theme = t;
                self.get_config().save_config(&self.widgets.prefs_window);
            }
            Msg::CustomTcpDumpBufferSizeToggled(_) | Msg::CustomTcpDumpBufferSizeValueChanged => {
                self.model.custom_tcpdump_buffer_size_kib =
                    if self.widgets.tcpdump_buf_size_cb.get_active() {
                        Some(self.widgets.tcpdump_buf_size_spinner.get_value() as usize)
                    } else {
                        None
                    };
                self.get_config().save_config(&self.widgets.prefs_window);
            }
        }
    }

    view! {
        #[name="prefs_window"]
        gtk::Window {
            titlebar: view! {
                gtk::HeaderBar {
                    title: Some("Preferences"),
                    show_close_button: true,
                }
            },
            property_default_width: 600,
            property_default_height: 200,
            gtk::Box {
                orientation: gtk::Orientation::Vertical,
                margin_top: 10,
                margin_start: 30,
                margin_end: 30,
                margin_bottom: 20,
                spacing: 6,
                gtk::CheckButton {
                    label: "Prefer dark theme",
                    active: self.model.prefer_dark_theme,
                    toggled(t) => Msg::DarkThemeToggled(t.get_active()),
                },
                gtk::Box {
                    orientation: gtk::Orientation::Horizontal,
                    spacing: 5,
                    visible: !win::is_flatpak(),
                    #[name="tcpdump_buf_size_cb"]
                    gtk::CheckButton {
                        label: "Custom tcpdump buffer size (KiB)",
                        active: self.model.custom_tcpdump_buffer_size_kib.is_some(),
                        toggled(t) => Msg::CustomTcpDumpBufferSizeToggled(t.get_active()),
                    },
                    #[name="tcpdump_buf_size_spinner"]
                    gtk::SpinButton {
                        numeric: true,
                        snap_to_ticks: true,
                        value_changed => Msg::CustomTcpDumpBufferSizeValueChanged,
                    },
                },
            }
        }
    }
}
