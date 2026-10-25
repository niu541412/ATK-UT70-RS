#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod firmware;
mod i18n;
mod protocol;
mod worker;

use eframe::egui;
use egui_plot::{Legend, Line, Plot};
use i18n::{Language, Message};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use worker::{Command, Event, TimedSample};

#[derive(Clone, Copy)]
enum Metric {
    Voltage,
    Current,
    Power,
    Dp,
    Dm,
    Resistance,
}

impl Metric {
    const ALL: [Self; 6] = [
        Self::Voltage,
        Self::Current,
        Self::Power,
        Self::Dp,
        Self::Dm,
        Self::Resistance,
    ];

    fn title(self, lang: Language) -> &'static str {
        match self {
            Self::Voltage => lang.pick("电压 V", "Voltage V"),
            Self::Current => lang.pick("电流 A", "Current A"),
            Self::Power => lang.pick("功率 W", "Power W"),
            Self::Dp => "D+ V",
            Self::Dm => "D− V",
            Self::Resistance => lang.pick("负载电阻 Ω", "Load resistance Ω"),
        }
    }

    fn color(self) -> egui::Color32 {
        match self {
            Self::Voltage => egui::Color32::from_rgb(60, 210, 110),
            Self::Current => egui::Color32::from_rgb(235, 85, 90),
            Self::Power => egui::Color32::from_rgb(170, 105, 235),
            Self::Dp => egui::Color32::from_rgb(70, 190, 235),
            Self::Dm => egui::Color32::from_rgb(240, 160, 65),
            Self::Resistance => egui::Color32::from_rgb(225, 205, 75),
        }
    }

    fn label_color(self, dark_mode: bool) -> egui::Color32 {
        if dark_mode {
            return self.color();
        }
        match self {
            Self::Voltage => egui::Color32::from_rgb(25, 130, 67),
            Self::Current => egui::Color32::from_rgb(175, 49, 54),
            Self::Power => egui::Color32::from_rgb(104, 58, 166),
            Self::Dp => egui::Color32::from_rgb(25, 111, 169),
            Self::Dm => egui::Color32::from_rgb(167, 91, 15),
            Self::Resistance => egui::Color32::from_rgb(132, 108, 12),
        }
    }

    fn value(self, sample: &protocol::Sample) -> Option<f64> {
        let value = match self {
            Self::Voltage => sample.voltage as f64,
            Self::Current => sample.current as f64,
            Self::Power => (sample.voltage * sample.current) as f64,
            Self::Dp => sample.dp as f64,
            Self::Dm => sample.dm as f64,
            Self::Resistance if sample.current > 0.001 => (sample.voltage / sample.current) as f64,
            Self::Resistance => return None,
        };
        value.is_finite().then_some(value)
    }
}

struct App {
    commands: Sender<Command>,
    events: Receiver<Event>,
    samples: VecDeque<TimedSample>,
    status: Message,
    language: Language,
    connecting: bool,
    connected: bool,
    bootloader: bool,
    flashing: bool,
    recording: bool,
    rate: u16,
    screen: u8,
    version: Option<protocol::VersionInfo>,
    firmware: Option<(PathBuf, firmware::Firmware)>,
    flash_confirmed: bool,
    progress: Option<(usize, usize)>,
    firmware_dialog_open: bool,
    settings_dialog_open: bool,
    trace_visible: [bool; 6],
}

impl App {
    fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        std::thread::spawn(move || worker::run(command_rx, event_tx));
        let _ = command_tx.send(Command::Connect);
        Self {
            commands: command_tx,
            events: event_rx,
            samples: VecDeque::new(),
            status: Message::new("正在连接 UT70…", "Connecting to UT70…"),
            language: Language::Chinese,
            connecting: true,
            connected: false,
            bootloader: false,
            flashing: false,
            recording: false,
            rate: 4,
            screen: 0,
            version: None,
            firmware: None,
            flash_confirmed: false,
            progress: None,
            firmware_dialog_open: false,
            settings_dialog_open: false,
            trace_visible: [true, true, false, false, false, false],
        }
    }

    fn receive_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Status(message) => self.status = message,
                Event::Connected(version, rate, rate_confirmed) => {
                    self.connecting = false;
                    self.connected = true;
                    self.bootloader = false;
                    self.rate = rate;
                    self.status = if !rate_confirmed {
                        Message::new(
                            format!("已连接，采样率查询未响应；暂按 {rate} SPS 解析，请手动设置"),
                            format!(
                                "Connected, but the sample rate query timed out. Using {rate} SPS until you set a rate."
                            ),
                        )
                    } else if version.is_some() {
                        Message::new(
                            format!("已连接；采样率 {rate} SPS"),
                            format!("Connected; sample rate {rate} SPS"),
                        )
                    } else {
                        Message::new(
                            format!("已连接；版本查询未响应，采样率已确认 {rate} SPS"),
                            format!(
                                "Connected; version query timed out, sample rate confirmed at {rate} SPS"
                            ),
                        )
                    };
                    self.version = version;
                }
                Event::Bootloader(version) => {
                    self.connecting = false;
                    self.connected = false;
                    self.bootloader = true;
                    self.samples.clear();
                    self.flash_confirmed = false;
                    self.version = Some(version);
                    self.status = Message::new(
                        "仪表处于升级模式，可选择本地 .atk 固件",
                        "The meter is in upgrade mode. Select a local .atk firmware file.",
                    );
                }
                Event::Disconnected => {
                    self.connecting = false;
                    self.connected = false;
                    self.bootloader = false;
                    self.version = None;
                    self.samples.clear();
                    self.flash_confirmed = false;
                    self.status = Message::new("已断开设备", "Meter disconnected");
                }
                Event::Rate(rate) => {
                    self.rate = rate;
                    self.status = Message::new(
                        format!("采样率已设为 {rate} SPS"),
                        format!("Sample rate set to {rate} SPS"),
                    );
                }
                Event::Screen(screen) => {
                    self.screen = screen;
                    self.status = Message::new(
                        format!("设备报告的界面编号：{screen}"),
                        format!("Meter reports screen ID {screen}"),
                    );
                }
                Event::Samples(batch) => {
                    self.samples.extend(batch);
                    if let Some(last) = self.samples.back() {
                        let threshold = last.seconds - 60.0;
                        while self
                            .samples
                            .front()
                            .is_some_and(|first| first.seconds < threshold)
                        {
                            self.samples.pop_front();
                        }
                    }
                }
                Event::Recording(value) => self.recording = value,
                Event::FlashProgress(done, total) => self.progress = Some((done, total)),
                Event::FlashDone => {
                    self.flashing = false;
                    self.bootloader = false;
                    self.version = None;
                    self.flash_confirmed = false;
                    self.status = Message::new(
                        "烧录完成；设备已重启并通过应用版本校验。点击连接开始测量。",
                        "Flashing finished. The meter rebooted and its application version was verified. Select Connect to measure.",
                    );
                }
                Event::Error(message) => {
                    self.connecting = false;
                    self.flashing = false;
                    self.status = message;
                }
            }
        }
    }

    fn latest(&self) -> Option<protocol::Sample> {
        self.samples.back().map(|item| item.sample)
    }

    fn chart_points(&self, metric: Metric) -> Vec<[f64; 2]> {
        // 每条曲线最多绘制约 2000 个点；CSV 仍记录收到的全部测量点。
        let width = self.samples.len().div_ceil(1000).max(1);
        let values: Vec<_> = self.samples.iter().collect();
        let mut out = Vec::with_capacity(values.len().min(2000));
        for bucket in values.chunks(width) {
            let mut valid = bucket
                .iter()
                .filter_map(|item| metric.value(&item.sample).map(|value| (*item, value)));
            let Some((mut low, mut low_value)) = valid.next() else {
                continue;
            };
            let mut high = low;
            let mut high_value = low_value;
            for (item, value) in valid {
                if value < low_value {
                    low = item;
                    low_value = value;
                }
                if value > high_value {
                    high = item;
                    high_value = value;
                }
            }
            let first = if low.seconds <= high.seconds {
                low
            } else {
                high
            };
            let second = if low.seconds <= high.seconds {
                high
            } else {
                low
            };
            out.push([first.seconds, metric.value(&first.sample).unwrap()]);
            if first.seconds != second.seconds {
                out.push([second.seconds, metric.value(&second.sample).unwrap()]);
            }
        }
        out
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.horizontal(|ui| {
            ui.heading("ATK UT70");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let settings = ui
                    .add_sized([36.0, 34.0], egui::Button::new(""))
                    .on_hover_text(lang.pick("设置", "Settings"));
                Self::draw_settings_icon(ui, settings.rect);
                if settings.clicked() {
                    self.settings_dialog_open = true;
                }
                if ui
                    .button(lang.pick("固件更新", "Firmware update"))
                    .clicked()
                {
                    self.firmware_dialog_open = true;
                }
                let present = self.connected || self.bootloader;
                let toggle = ui
                    .add_enabled(
                        !self.flashing && !self.connecting,
                        egui::Button::new("").min_size(egui::vec2(40.0, 34.0)),
                    )
                    .on_hover_text(if present {
                        lang.pick("断开设备", "Disconnect meter")
                    } else {
                        lang.pick("连接设备", "Connect meter")
                    });
                Self::draw_connection_icon(ui, toggle.rect, present, self.connecting);
                if toggle.clicked() {
                    if present {
                        let _ = self.commands.send(Command::Disconnect);
                    } else {
                        let _ = self.commands.send(Command::Connect);
                        self.connecting = true;
                        self.status = Message::new("正在连接…", "Connecting…");
                    }
                }
            });
        });
        ui.add(egui::Label::new(self.status.text(lang)).truncate())
            .on_hover_text(self.status.text(lang));
        self.device_details(ui);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.connected && !self.recording,
                    egui::Button::new(lang.pick("开始 CSV 记录", "Start CSV recording")),
                )
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .save_file()
                {
                    let _ = self.commands.send(Command::StartCsv(path));
                }
            }
            if ui
                .add_enabled(
                    self.recording,
                    egui::Button::new(lang.pick("停止记录", "Stop recording")),
                )
                .clicked()
            {
                let _ = self.commands.send(Command::StopCsv);
            }
        });
        ui.horizontal(|ui| {
            let mut selected_rate = self.rate;
            ui.add_enabled_ui(self.connected && !self.flashing && !self.recording, |ui| {
                egui::ComboBox::from_label(lang.pick("采样率", "Sample rate"))
                    .selected_text(format!("{} SPS", self.rate))
                    .show_ui(ui, |ui| {
                        for rate in protocol::RATES {
                            ui.selectable_value(&mut selected_rate, rate, format!("{rate} SPS"));
                        }
                    });
            });
            if selected_rate != self.rate {
                let _ = self.commands.send(Command::SetRate(selected_rate));
            }
            let mut selected_screen = self.screen;
            ui.add_enabled_ui(self.connected && !self.flashing && !self.recording, |ui| {
                egui::ComboBox::from_label(lang.pick("仪表界面", "Meter screen"))
                    .selected_text(
                        protocol::SCREENS
                            .iter()
                            .find(|(id, _, _)| *id == self.screen)
                            .map(|(_, zh, en)| lang.pick(zh, en))
                            .unwrap_or(lang.pick("未知", "Unknown")),
                    )
                    .show_ui(ui, |ui| {
                        for (id, zh, en) in protocol::SCREENS {
                            ui.selectable_value(&mut selected_screen, *id, lang.pick(zh, en));
                        }
                    });
            });
            if selected_screen != self.screen {
                let _ = self.commands.send(Command::SetScreen(selected_screen));
            }
        });
        ui.add_space(6.0);
        self.readings(ui);
    }

    fn device_details(&self, ui: &mut egui::Ui) {
        let lang = self.language;
        let version = self.version.as_ref();
        let mode = if self.connecting {
            lang.pick("连接中", "Connecting")
        } else if self.bootloader {
            lang.pick("升级模式", "Upgrade mode")
        } else if self.connected {
            lang.pick("应用模式", "Application mode")
        } else {
            lang.pick("未连接", "Disconnected")
        };
        let hardware = version.map_or("—".into(), |v| {
            format!("{}.{}", v.hardware / 10, v.hardware % 10)
        });
        let application = version.filter(|v| !v.boot_mode).map_or("—".into(), |v| {
            format!("V{}.{}", v.application / 10, v.application % 10)
        });
        let boot = version.map_or("—".into(), |v| v.bootloader.to_string());
        let serial = version.map_or("—".into(), |v| v.serial.clone());
        let fields = [
            (lang.pick("状态", "Status"), mode.to_owned()),
            (lang.pick("硬件", "Hardware"), hardware),
            (lang.pick("应用", "Application"), application),
            ("Boot", boot),
            (lang.pick("序列号", "Serial"), serial),
        ];
        ui.columns(5, |columns| {
            for (column, (label, value)) in columns.iter_mut().zip(fields) {
                column.label(egui::RichText::new(label).small().weak());
                column.label(egui::RichText::new(value).strong());
            }
        });
    }

    fn draw_connection_icon(ui: &egui::Ui, rect: egui::Rect, present: bool, connecting: bool) {
        let painter = ui.painter_at(rect);
        let center = rect.center();
        let color = if connecting {
            egui::Color32::from_rgb(230, 180, 70)
        } else if present {
            egui::Color32::from_rgb(80, 215, 135)
        } else {
            ui.visuals().text_color()
        };
        let stroke = egui::Stroke::new(2.0, color);
        let x = center.x;
        let y = center.y;
        let shift = if present { 6.0 } else { -2.0 };
        painter.line_segment(
            [egui::pos2(x - 17.0, y), egui::pos2(x - 9.0 + shift, y)],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(x - 9.0 + shift, y - 6.0),
                egui::pos2(x - 9.0 + shift, y + 6.0),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(x - 9.0 + shift, y - 6.0),
                egui::pos2(x - 2.0 + shift, y - 6.0),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(x - 9.0 + shift, y + 6.0),
                egui::pos2(x - 2.0 + shift, y + 6.0),
            ],
            stroke,
        );
        for dy in [-3.0, 3.0] {
            painter.line_segment(
                [
                    egui::pos2(x - 2.0 + shift, y + dy),
                    egui::pos2(x + 2.0 + shift, y + dy),
                ],
                stroke,
            );
        }
        painter.line_segment(
            [egui::pos2(x + 8.0, y - 6.0), egui::pos2(x + 8.0, y + 6.0)],
            stroke,
        );
        painter.line_segment([egui::pos2(x + 8.0, y), egui::pos2(x + 17.0, y)], stroke);
    }

    fn draw_settings_icon(ui: &egui::Ui, rect: egui::Rect) {
        let painter = ui.painter_at(rect);
        let center = rect.center();
        let stroke = egui::Stroke::new(1.8, ui.visuals().text_color());
        painter.circle_stroke(center, 7.0, stroke);
        painter.circle_stroke(center, 2.3, stroke);
        for index in 0..8 {
            let angle = index as f32 * std::f32::consts::TAU / 8.0;
            let direction = egui::vec2(angle.cos(), angle.sin());
            painter.line_segment(
                [center + direction * 8.0, center + direction * 11.0],
                stroke,
            );
        }
    }

    fn reading(ui: &mut egui::Ui, title: &str, value: String, color: egui::Color32) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 94.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::CornerRadius::same(12), color);
        painter.text(
            rect.left_top() + egui::vec2(15.0, 12.0),
            egui::Align2::LEFT_TOP,
            title,
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
        painter.text(
            rect.left_top() + egui::vec2(15.0, 39.0),
            egui::Align2::LEFT_TOP,
            value,
            egui::FontId::monospace(30.0),
            egui::Color32::WHITE,
        );
    }

    fn readings(&self, ui: &mut egui::Ui) {
        let lang = self.language;
        let latest = self.latest();
        ui.columns(3, |columns| {
            Self::reading(
                &mut columns[0],
                lang.pick("电压", "Voltage"),
                latest.map_or("—".into(), |s| format!("{:.3} V", s.voltage)),
                egui::Color32::from_rgb(24, 117, 75),
            );
            Self::reading(
                &mut columns[1],
                lang.pick("电流", "Current"),
                latest.map_or("—".into(), |s| format!("{:.3} A", s.current)),
                egui::Color32::from_rgb(168, 51, 65),
            );
            Self::reading(
                &mut columns[2],
                lang.pick("功率", "Power"),
                latest.map_or("—".into(), |s| format!("{:.3} W", s.voltage * s.current)),
                egui::Color32::from_rgb(108, 65, 167),
            );
        });
        if let Some(s) = latest {
            ui.horizontal(|ui| {
                ui.colored_label(
                    Metric::Dp.label_color(ui.visuals().dark_mode),
                    format!("D+ {:.4} V", s.dp),
                );
                ui.colored_label(
                    Metric::Dm.label_color(ui.visuals().dark_mode),
                    format!("D− {:.4} V", s.dm),
                );
            });
        } else {
            ui.label(lang.pick("暂无测量数据", "No measurement data yet"));
        }
    }

    fn plots(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.horizontal_wrapped(|ui| {
            for (index, metric) in Metric::ALL.iter().enumerate() {
                ui.checkbox(
                    &mut self.trace_visible[index],
                    egui::RichText::new(metric.title(lang))
                        .color(metric.label_color(ui.visuals().dark_mode)),
                );
            }
        });
        let visible = self.trace_visible;
        let plot_height = (ui.available_height() - 32.0).max(150.0);
        Plot::new("measurements_plot")
            .width(ui.available_width())
            .height(plot_height)
            .legend(Legend::default())
            .x_axis_label(lang.pick("秒", "Seconds"))
            .y_axis_label(lang.pick("数值（单位见图例）", "Value (units in legend)"))
            .show(ui, |plot| {
                for (index, metric) in Metric::ALL.iter().enumerate() {
                    if visible[index] {
                        plot.line(
                            Line::new(metric.title(lang), self.chart_points(*metric))
                                .color(metric.color())
                                .width(2.0),
                        );
                    }
                }
            });
        ui.small(lang.pick(
            "曲线显示最近 60 秒。负载电阻按 V/I 计算，电流不高于 1 mA 时不绘制；它不是线阻。CSV 记录全部测量点。",
            "Charts show the latest 60 seconds. Load resistance is V/I and is omitted at or below 1 mA; it is not cable resistance. CSV stores all samples.",
        ));
    }

    fn flashing_controls(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.heading(lang.pick("本地固件升级", "Local firmware upgrade"));
        ui.label(lang.pick(
            "先拔下侧面 HID 线，按住仪表右键重新插入，使仪表进入升级模式。选择适配硬件版本的官方 .atk 文件。",
            "Unplug the side HID cable, hold the meter's right button while reconnecting it, then select an official .atk file for this hardware revision.",
        ));
        if ui
            .add_enabled(
                !self.flashing,
                egui::Button::new(lang.pick("选择 .atk 文件", "Select .atk file")),
            )
            .clicked()
        {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(lang.pick("ATK 固件", "ATK firmware"), &["atk"])
                .pick_file()
            {
                match firmware::Firmware::read(&path) {
                    Ok(info) => {
                        self.firmware = Some((path, info));
                        self.flash_confirmed = false;
                    }
                    Err(e) => {
                        self.firmware = None;
                        self.status = Message::new(
                            format!("固件文件无效：{e}"),
                            format!("Invalid firmware file: {e}"),
                        );
                    }
                }
            }
        }
        if let Some((path, info)) = &self.firmware {
            ui.label(path.display().to_string());
            ui.label(match lang {
                Language::Chinese => format!(
                    "硬件 {}.{} · 目标应用 V{}.{} · 镜像 {} 字节 · 构建日期 {}",
                    info.hardware / 10,
                    info.hardware % 10,
                    info.application / 10,
                    info.application % 10,
                    info.image_size,
                    info.build_date,
                ),
                Language::English => format!(
                    "Hardware {}.{} · Target app V{}.{} · Image {} bytes · Built {}",
                    info.hardware / 10,
                    info.hardware % 10,
                    info.application / 10,
                    info.application % 10,
                    info.image_size,
                    info.build_date,
                ),
            });
            ui.checkbox(
                &mut self.flash_confirmed,
                lang.pick(
                    "确认设备已进入升级模式，烧录期间保持供电和 HID 连接",
                    "I confirm the meter is in upgrade mode and will remain powered and connected",
                ),
            );
            if ui
                .add_enabled(
                    self.bootloader && self.flash_confirmed && !self.flashing,
                    egui::Button::new(lang.pick("开始烧录", "Start flashing")),
                )
                .clicked()
            {
                let _ = self.commands.send(Command::Flash(path.clone()));
                self.flashing = true;
                self.progress = None;
                self.status = Message::new("正在准备升级…", "Preparing firmware upgrade…");
            }
        }
        if let Some((done, total)) = self.progress {
            ui.add(
                egui::ProgressBar::new(done as f32 / total as f32).text(format!("{done}/{total}")),
            );
        }
        if self.flashing {
            ui.label(lang.pick(
                "烧录中，请保持程序运行和设备连接。",
                "Flashing in progress. Keep this app open and the meter connected.",
            ));
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive_events();
        if self.flashing && ui.ctx().input(|input| input.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        egui::Panel::top("main_header")
            .resizable(false)
            .show(ui, |ui| self.header(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            self.plots(ui);
        });
        if self.settings_dialog_open {
            let mut open = true;
            egui::Window::new(self.language.pick("设置", "Settings"))
                .open(&mut open)
                .default_width(280.0)
                .show(ui.ctx(), |ui| {
                    egui::ComboBox::from_label(self.language.pick("语言", "Language"))
                        .selected_text(match self.language {
                            Language::Chinese => "简体中文",
                            Language::English => "English",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.language, Language::Chinese, "简体中文");
                            ui.selectable_value(&mut self.language, Language::English, "English");
                        });
                });
            self.settings_dialog_open = open;
        }
        if self.firmware_dialog_open {
            let mut open = true;
            egui::Window::new(self.language.pick("固件更新", "Firmware update"))
                .open(&mut open)
                .default_size([600.0, 300.0])
                .scroll(true)
                .show(ui.ctx(), |ui| self.flashing_controls(ui));
            self.firmware_dialog_open = open;
        }
        ui.ctx()
            .request_repaint_after(if self.connected || self.flashing {
                Duration::from_millis(50)
            } else {
                Duration::from_millis(250)
            });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
    }
}

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "ATK UT70 RS",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([980.0, 650.0])
                .with_min_inner_size([720.0, 600.0])
                .with_icon(
                    eframe::icon_data::from_png_bytes(include_bytes!("../assets/atk-ut70-rs.png"))
                        .expect("内置图标应为有效 PNG"),
                ),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(App::new()))),
    )
}
