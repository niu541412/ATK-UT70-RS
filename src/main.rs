#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod firmware;
mod i18n;
mod protocol;
mod settings;
mod worker;

use eframe::egui;
use egui_plot::{CoordinatesFormatter, Corner, Legend, Line, Plot};
use i18n::{Language, Message};
use settings::Settings;
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use worker::{Command, Event, TimedSample};

struct IconTextures {
    plugs: egui::TextureHandle,
    plugs_connected: egui::TextureHandle,
    settings: egui::TextureHandle,
    firmware: egui::TextureHandle,
    protocol_scan: egui::TextureHandle,
    sun: egui::TextureHandle,
    moon: egui::TextureHandle,
}

impl IconTextures {
    fn new(ctx: &egui::Context) -> Self {
        fn load(ctx: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
            let icon = eframe::icon_data::from_png_bytes(bytes).expect("内置图标应为有效 PNG");
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
        }
        Self {
            plugs: load(ctx, "plugs", include_bytes!("../assets/icons/plugs.png")),
            plugs_connected: load(
                ctx,
                "plugs-connected",
                include_bytes!("../assets/icons/plugs-connected.png"),
            ),
            settings: load(
                ctx,
                "gear-six",
                include_bytes!("../assets/icons/gear-six.png"),
            ),
            firmware: load(
                ctx,
                "file-arrow-up",
                include_bytes!("../assets/icons/file-arrow-up.png"),
            ),
            protocol_scan: load(
                ctx,
                "list-magnifying-glass",
                include_bytes!("../assets/icons/list-magnifying-glass.png"),
            ),
            sun: load(ctx, "sun", include_bytes!("../assets/icons/sun.png")),
            moon: load(ctx, "moon", include_bytes!("../assets/icons/moon.png")),
        }
    }
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut inter = egui::FontData::from_static(include_bytes!("../assets/fonts/Inter.ttf"));
    inter.tweak.coords.push(b"wght", 540.0);
    fonts.font_data.insert("Inter".into(), Arc::new(inter));
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .expect("内置比例字体系列应存在")
        .insert(0, "Inter".into());
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|style| {
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(20.0));
    });
}

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
            Self::Resistance => egui::Color32::from_rgb(232, 236, 241),
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
            Self::Resistance => egui::Color32::BLACK,
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
    started_at: Instant,
    commands: Sender<Command>,
    events: Receiver<Event>,
    icons: IconTextures,
    samples: VecDeque<TimedSample>,
    chart_origin: Option<f64>,
    paused: bool,
    window_secs: u16,
    cursor_values: bool,
    status: Message,
    language: Language,
    connecting: bool,
    connected: bool,
    bootloader: bool,
    flashing: bool,
    recording: bool,
    rate: u16,
    screen: u8,
    pending_screen: Option<u8>,
    version: Option<protocol::VersionInfo>,
    firmware: Option<(PathBuf, firmware::Firmware)>,
    flash_confirmed: bool,
    progress: Option<(usize, usize)>,
    firmware_dialog_open: bool,
    settings_dialog_open: bool,
    protocol_dialog_open: bool,
    protocol_panel_expanded_window: bool,
    selected_protocol: Option<u8>,
    scanning_protocols: bool,
    protocol_results: BTreeMap<u8, ProtocolResult>,
    logs: VecDeque<(String, Message)>,
    trace_visible: [bool; 6],
}

#[derive(Default)]
struct ProtocolResult {
    mask: Option<u32>,
    description: Option<String>,
    supported: Option<bool>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        configure_fonts(ctx);
        let settings = Settings::load();
        ctx.set_theme(if settings.dark_mode {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        std::thread::spawn(move || worker::run(command_rx, event_tx));
        let _ = command_tx.send(Command::Connect);
        Self {
            started_at: Instant::now(),
            commands: command_tx,
            events: event_rx,
            icons: IconTextures::new(ctx),
            samples: VecDeque::new(),
            chart_origin: None,
            paused: false,
            window_secs: settings.window_secs,
            cursor_values: settings.cursor_values,
            status: Message::new("正在连接 UT70…", "Connecting to UT70…"),
            language: settings.language,
            connecting: true,
            connected: false,
            bootloader: false,
            flashing: false,
            recording: false,
            rate: 4,
            screen: 0,
            pending_screen: None,
            version: None,
            firmware: None,
            flash_confirmed: false,
            progress: None,
            firmware_dialog_open: false,
            settings_dialog_open: false,
            protocol_dialog_open: false,
            protocol_panel_expanded_window: false,
            selected_protocol: None,
            scanning_protocols: false,
            protocol_results: BTreeMap::new(),
            logs: VecDeque::from([(
                Self::local_time(),
                Message::new("正在连接 UT70…", "Connecting to UT70…"),
            )]),
            trace_visible: [true, true, false, false, false, false],
        }
    }

    fn receive_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Status(message) => {
                    self.log(message.clone());
                    self.status = message;
                }
                Event::Connected(version, rate, rate_confirmed) => {
                    self.samples.clear();
                    self.chart_origin = None;
                    self.paused = false;
                    self.protocol_results.clear();
                    self.scanning_protocols = false;
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
                    self.log(self.status.clone());
                }
                Event::Bootloader(version) => {
                    self.connecting = false;
                    self.connected = false;
                    self.bootloader = true;
                    self.samples.clear();
                    self.chart_origin = None;
                    self.paused = false;
                    self.flash_confirmed = false;
                    self.version = Some(version);
                    self.status = Message::new(
                        "仪表处于升级模式，可选择本地 .atk 固件",
                        "The meter is in upgrade mode. Select a local .atk firmware file.",
                    );
                    self.log(self.status.clone());
                }
                Event::Disconnected => {
                    self.pending_screen = None;
                    self.scanning_protocols = false;
                    self.protocol_results.clear();
                    self.connecting = false;
                    self.connected = false;
                    self.bootloader = false;
                    self.version = None;
                    self.samples.clear();
                    self.chart_origin = None;
                    self.paused = false;
                    self.flash_confirmed = false;
                    self.status = Message::new("已断开设备", "Meter disconnected");
                    self.log(self.status.clone());
                }
                Event::Rate(rate) => {
                    self.rate = rate;
                    self.status = Message::new(
                        format!("采样率已设为 {rate} SPS"),
                        format!("Sample rate set to {rate} SPS"),
                    );
                    self.log(self.status.clone());
                }
                Event::Screen(screen) => {
                    if self.screen != screen {
                        self.screen = screen;
                        let (chinese, english) = protocol::screen_names(screen);
                        self.status = Message::new(
                            format!("设备报告的界面：{chinese}"),
                            format!("Meter reports screen: {english}"),
                        );
                        self.log(self.status.clone());
                    }
                }
                Event::ScreenCommandSent(screen) => {
                    self.screen = screen;
                    let (_, chinese, english) = protocol::SCREENS
                        .iter()
                        .find(|(id, _, _)| *id == screen)
                        .expect("已发送的界面编号应在列表中");
                    self.status = Message::new(
                        format!("已发送{chinese}指令；检测状态请以仪表显示为准"),
                        format!("Sent {english} command; check the meter for its actual state"),
                    );
                    self.log(self.status.clone());
                }
                Event::ProtocolScanStarted => {
                    self.scanning_protocols = true;
                    self.protocol_results.clear();
                    self.status = Message::new("正在检测供电协议…", "Scanning charging protocols…");
                    self.log(self.status.clone());
                }
                Event::ProtocolReport(report) => match report {
                    protocol::ProtocolReport::Mask { kind, mask } => {
                        self.protocol_results.entry(kind).or_default().mask = Some(mask);
                    }
                    protocol::ProtocolReport::Description {
                        kind,
                        supported,
                        text,
                    } => {
                        let summary = text.trim().to_owned();
                        let result = self.protocol_results.entry(kind).or_default();
                        result.supported = Some(supported);
                        result.description = Some(text);
                        self.log(Message::new(
                            format!("{}：{summary}", protocol::protocol_name(kind)),
                            format!("{}: {summary}", protocol::protocol_name(kind)),
                        ));
                    }
                },
                Event::ProtocolScanFinished => {
                    self.scanning_protocols = false;
                    self.status = Message::new("协议检测已结束", "Protocol scan finished");
                    self.log(self.status.clone());
                }
                Event::Samples(batch) => {
                    if self.paused {
                        continue;
                    }
                    if self.chart_origin.is_none() {
                        self.chart_origin = batch.first().map(|item| item.seconds);
                    }
                    self.samples.extend(batch);
                    if let Some(last) = self.samples.back() {
                        let threshold = last.seconds - 600.0;
                        while self
                            .samples
                            .front()
                            .is_some_and(|first| first.seconds < threshold)
                        {
                            self.samples.pop_front();
                        }
                    }
                }
                Event::Recording(value) => {
                    self.recording = value;
                    self.log(if value {
                        Message::new("CSV 记录已开始", "CSV recording started")
                    } else {
                        Message::new("CSV 记录已停止", "CSV recording stopped")
                    });
                }
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
                    self.log(self.status.clone());
                }
                Event::Error(message) => {
                    self.connecting = false;
                    self.flashing = false;
                    self.status = message;
                    self.log(self.status.clone());
                }
            }
        }
    }

    fn log(&mut self, message: Message) {
        self.logs.push_back((Self::local_time(), message));
        while self.logs.len() > 60 {
            self.logs.pop_front();
        }
    }

    fn local_time() -> String {
        chrono::Local::now().format("%H:%M:%S").to_string()
    }

    fn save_settings(&self, ctx: &egui::Context) {
        Settings {
            language: self.language,
            dark_mode: ctx.theme() == egui::Theme::Dark,
            cursor_values: self.cursor_values,
            window_secs: self.window_secs,
        }
        .save();
    }

    fn restart_measurement(&mut self) {
        self.samples.clear();
        self.chart_origin = None;
        self.paused = false;
        self.log(Message::new(
            "曲线已重新开始；CSV 记录不受影响",
            "Chart restarted; CSV recording continues",
        ));
    }

    fn set_protocol_panel(&mut self, ctx: &egui::Context, open: bool) {
        if self.protocol_dialog_open == open {
            return;
        }
        self.protocol_dialog_open = open;
        if let Some(rect) = ctx.input(|input| input.viewport().inner_rect) {
            if open && !self.protocol_panel_expanded_window {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                    rect.size() + egui::vec2(300.0, 0.0),
                ));
                self.protocol_panel_expanded_window = true;
            } else if !open && self.protocol_panel_expanded_window {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                    rect.size() - egui::vec2(300.0, 0.0),
                ));
                self.protocol_panel_expanded_window = false;
            }
        }
    }

    fn protocol_color(kind: u8) -> egui::Color32 {
        match kind {
            0 | 12 => egui::Color32::from_rgb(32, 137, 100), // BC1.2 / Apple
            1 | 2 => egui::Color32::from_rgb(172, 105, 26),  // QC
            3 | 4 | 13 => egui::Color32::from_rgb(39, 112, 173), // FCP / SCP / VFCP
            5 => egui::Color32::from_rgb(126, 75, 166),      // AFC
            6 | 7 => egui::Color32::from_rgb(158, 73, 139),  // PE
            8 | 10 => egui::Color32::from_rgb(179, 66, 84),  // VOOC / DASH
            9 | 11 => egui::Color32::from_rgb(76, 87, 166),  // PD / E-Marker
            20 => egui::Color32::from_rgb(26, 131, 147),     // UFCS
            _ => egui::Color32::from_rgb(83, 93, 109),
        }
    }

    fn latest(&self) -> Option<protocol::Sample> {
        self.samples.back().map(|item| item.sample)
    }

    fn ripple_vpp_mv(&self) -> Option<f64> {
        if !self.connected {
            return None;
        }
        ripple_vpp_mv(&self.samples)
    }

    fn chart_points(&self, metric: Metric) -> Vec<[f64; 2]> {
        // 每条曲线最多绘制约 2000 个点；CSV 仍记录收到的全部测量点。
        let Some(last) = self.samples.back() else {
            return Vec::new();
        };
        let start = lower_bound_time(&self.samples, last.seconds - self.window_secs as f64);
        let width = (self.samples.len() - start).div_ceil(1000).max(1);
        let mut out = Vec::with_capacity((self.samples.len() - start).min(2000));
        let origin = self.chart_origin.unwrap_or(0.0);
        if let Some((item, value)) = self
            .samples
            .range(start..)
            .find_map(|item| metric.value(&item.sample).map(|value| (item, value)))
        {
            out.push([item.seconds - origin, value]);
        }
        for index in (start..self.samples.len()).step_by(width) {
            let bucket = self
                .samples
                .range(index..(index + width).min(self.samples.len()));
            let mut valid =
                bucket.filter_map(|item| metric.value(&item.sample).map(|value| (item, value)));
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
            out.push([first.seconds - origin, metric.value(&first.sample).unwrap()]);
            if first.seconds != second.seconds {
                out.push([
                    second.seconds - origin,
                    metric.value(&second.sample).unwrap(),
                ]);
            }
        }
        if let Some((item, value)) = self
            .samples
            .iter()
            .rev()
            .find_map(|item| metric.value(&item.sample).map(|value| (item, value)))
        {
            let point = [item.seconds - origin, value];
            if out.last().is_none_or(|previous| previous[0] < point[0]) {
                out.push(point);
            }
        }
        out
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        let settings_icon = self.icons.settings.id();
        let firmware_icon = self.icons.firmware.id();
        let protocol_scan_icon = self.icons.protocol_scan.id();
        let sun_icon = self.icons.sun.id();
        let moon_icon = self.icons.moon.id();
        let plugs_icon = self.icons.plugs.id();
        let plugs_connected_icon = self.icons.plugs_connected.id();
        ui.horizontal(|ui| {
            ui.heading(lang.pick("正点原子UT70", "ALIENTEK UT70"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tint = ui.visuals().text_color();
                let icon_size = egui::vec2(20.0, 20.0);
                let button_size = egui::vec2(40.0, 36.0);
                let settings = ui
                    .add(
                        egui::Button::image(
                            egui::Image::new((settings_icon, icon_size)).tint(tint),
                        )
                        .min_size(button_size),
                    )
                    .on_hover_text(lang.pick("设置", "Settings"));
                if settings.clicked() {
                    self.settings_dialog_open = true;
                }
                let dark = ui.visuals().dark_mode;
                let theme = ui
                    .add(
                        egui::Button::image(
                            egui::Image::new((if dark { sun_icon } else { moon_icon }, icon_size))
                                .tint(tint),
                        )
                        .min_size(button_size),
                    )
                    .on_hover_text(if dark {
                        lang.pick("切换到浅色模式", "Switch to light mode")
                    } else {
                        lang.pick("切换到深色模式", "Switch to dark mode")
                    });
                if theme.clicked() {
                    ui.ctx().set_theme(if dark {
                        egui::Theme::Light
                    } else {
                        egui::Theme::Dark
                    });
                    self.save_settings(ui.ctx());
                }
                let firmware = ui
                    .add(
                        egui::Button::image(
                            egui::Image::new((firmware_icon, icon_size)).tint(tint),
                        )
                        .min_size(button_size),
                    )
                    .on_hover_text(lang.pick("固件更新", "Firmware update"));
                if firmware.clicked() {
                    self.firmware_dialog_open = true;
                }
                let scan = ui
                    .add(
                        egui::Button::image(
                            egui::Image::new((protocol_scan_icon, icon_size)).tint(
                                if self.protocol_dialog_open {
                                    egui::Color32::from_rgb(31, 143, 203)
                                } else {
                                    tint
                                },
                            ),
                        )
                        .min_size(button_size),
                    )
                    .on_hover_text(lang.pick("协议检测", "Protocol scan"));
                if scan.clicked() {
                    self.set_protocol_panel(ui.ctx(), !self.protocol_dialog_open);
                }
                let present = self.connected || self.bootloader;
                let connection_color = if self.connecting {
                    egui::Color32::from_rgb(194, 141, 36)
                } else if present {
                    egui::Color32::from_rgb(23, 151, 91)
                } else {
                    tint
                };
                let toggle = ui
                    .add_enabled(
                        !self.flashing && !self.connecting,
                        egui::Button::image(
                            egui::Image::new((
                                if present {
                                    plugs_connected_icon
                                } else {
                                    plugs_icon
                                },
                                icon_size,
                            ))
                            .tint(connection_color),
                        )
                        .min_size(button_size),
                    )
                    .on_hover_text(if present {
                        lang.pick("断开设备", "Disconnect meter")
                    } else {
                        lang.pick("连接设备", "Connect meter")
                    });
                if toggle.clicked() {
                    if present {
                        let _ = self.commands.send(Command::Disconnect);
                    } else {
                        let _ = self.commands.send(Command::Connect);
                        self.connecting = true;
                        self.status = Message::new("正在连接…", "Connecting…");
                        self.log(self.status.clone());
                    }
                }
            });
        });
        ui.add(egui::Label::new(self.status.text(lang)).truncate())
            .on_hover_text(self.status.text(lang));
        let total_width = ui.available_width();
        let controls_width = (total_width * 0.58).max(440.0).min(total_width - 260.0);
        let log_width = total_width - controls_width - 8.0;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(controls_width, 151.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    self.device_details(ui);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.connected,
                                egui::Button::new(lang.pick("重新开始", "Restart")),
                            )
                            .on_hover_text(lang.pick(
                                "清空曲线并重新计时；CSV 记录继续进行",
                                "Clear the chart and restart its clock; CSV recording continues",
                            ))
                            .clicked()
                        {
                            self.restart_measurement();
                        }
                        if ui
                            .add_enabled(
                                self.connected,
                                egui::Button::new(if self.paused {
                                    lang.pick("继续", "Resume")
                                } else {
                                    lang.pick("暂停", "Pause")
                                }),
                            )
                            .on_hover_text(lang.pick(
                                "只暂停界面显示；CSV 仍会记录收到的所有采样点",
                                "Pause the display only; CSV continues recording every sample",
                            ))
                            .clicked()
                        {
                            self.paused = !self.paused;
                            self.log(if self.paused {
                                Message::new("显示已暂停", "Display paused")
                            } else {
                                Message::new("显示已继续", "Display resumed")
                            });
                        }
                        ui.add(egui::Separator::default().vertical().spacing(12.0));
                        if ui
                            .add_enabled(
                                self.connected || self.recording,
                                egui::Button::new(if self.recording {
                                    lang.pick("停止 CSV 记录", "Stop CSV recording")
                                } else {
                                    lang.pick("开始 CSV 记录", "Start CSV recording")
                                }),
                            )
                            .clicked()
                        {
                            if self.recording {
                                let _ = self.commands.send(Command::StopCsv);
                            } else if let Some(path) = rfd::FileDialog::new()
                                .add_filter("CSV", &["csv"])
                                .save_file()
                            {
                                let _ = self.commands.send(Command::StartCsv(path));
                            }
                        }
                    });
                    ui.add_space(7.0);
                    ui.horizontal(|ui| {
                        let mut selected_rate = self.rate;
                        ui.add_enabled_ui(
                            self.connected
                                && !self.flashing
                                && !self.recording
                                && !self.scanning_protocols,
                            |ui| {
                                ui.label(lang.pick("采样率", "Sample rate"));
                                egui::ComboBox::from_id_salt("sample_rate")
                                    .selected_text(format!("{} SPS", self.rate))
                                    .show_ui(ui, |ui| {
                                        for rate in protocol::RATES {
                                            ui.selectable_value(
                                                &mut selected_rate,
                                                rate,
                                                format!("{rate} SPS"),
                                            );
                                        }
                                    });
                            },
                        );
                        if selected_rate != self.rate {
                            let _ = self.commands.send(Command::SetRate(selected_rate));
                        }
                        ui.add(egui::Separator::default().vertical().spacing(15.0));
                        let mut selected_screen = self.screen;
                        ui.add_enabled_ui(
                            self.connected
                                && !self.flashing
                                && !self.recording
                                && !self.scanning_protocols,
                            |ui| {
                                ui.label(lang.pick("仪表界面", "Meter screen"));
                                egui::ComboBox::from_id_salt("meter_screen")
                                    .width(170.0)
                                    .height(280.0)
                                    .selected_text({
                                        let (zh, en) = protocol::screen_names(self.screen);
                                        lang.pick(zh, en)
                                    })
                                    .show_ui(ui, |ui| {
                                        for (id, zh, en) in protocol::SCREENS {
                                            ui.selectable_value(
                                                &mut selected_screen,
                                                *id,
                                                lang.pick(zh, en),
                                            );
                                        }
                                    });
                            },
                        );
                        if selected_screen != self.screen {
                            if matches!(selected_screen, 5 | 20) {
                                self.pending_screen = Some(selected_screen);
                            } else {
                                let _ = self.commands.send(Command::SetScreen(selected_screen));
                            }
                        }
                    });
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(log_width, 151.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.event_log(ui, log_width, 151.0),
            );
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
        let active_protocol = if self.connected {
            lang.pick("未识别", "Unknown")
        } else {
            "—"
        };
        let fields = [
            (lang.pick("状态", "Status"), mode.to_owned()),
            (lang.pick("硬件", "Hardware"), hardware),
            (lang.pick("应用", "Application"), application),
            ("Boot", boot),
            (lang.pick("序列号", "Serial"), serial),
            (
                lang.pick("当前协议", "Active protocol"),
                active_protocol.to_owned(),
            ),
        ];
        ui.columns(3, |columns| {
            for (index, (label, value)) in fields.into_iter().enumerate() {
                let column = &mut columns[index % 3];
                let label_response = column.label(egui::RichText::new(label).small());
                let value_response = column.add(
                    egui::Label::new(egui::RichText::new(value).strong()).truncate(),
                );
                if index == 5 {
                    let hint = lang.pick(
                        "普通测量帧未上报正在使用的协议；扫描结果只是可用协议列表。",
                        "Measurement frames do not report the active protocol; scan results list supported protocols.",
                    );
                    label_response.on_hover_text(hint);
                    value_response.on_hover_text(hint);
                }
            }
        });
    }

    fn event_log(&self, ui: &mut egui::Ui, width: f32, height: f32) {
        let lang = self.language;
        let group = ui.group(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(width - 18.0);
                ui.set_min_height(height - 18.0);
                ui.add_space(10.0);
                egui::ScrollArea::vertical()
                    .id_salt("event_log")
                    .max_height(height - 31.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for (clock, message) in &self.logs {
                            ui.add(
                                egui::Label::new(format!("{} {}", clock, message.text(lang)))
                                    .truncate(),
                            )
                            .on_hover_text(message.text(lang));
                        }
                    });
            });
        });
        ui.painter().text(
            group.response.rect.right_top() + egui::vec2(-8.0, 5.0),
            egui::Align2::RIGHT_TOP,
            lang.pick("日志", "Log"),
            egui::FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        );
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

    fn secondary_reading_row(
        ui: &mut egui::Ui,
        label: &str,
        value: String,
        unit: &str,
        color: egui::Color32,
        hint: Option<&str>,
    ) {
        let label_response = ui.colored_label(color, label);
        if let Some(hint) = hint {
            label_response.on_hover_text(hint);
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(78.0, 18.0), egui::Sense::hover());
        ui.painter().text(
            rect.right_center(),
            egui::Align2::RIGHT_CENTER,
            value,
            egui::FontId::monospace(14.0),
            color,
        );
        ui.colored_label(color, unit);
        ui.end_row();
    }

    fn readings(&self, ui: &mut egui::Ui) {
        let lang = self.language;
        let latest = self.latest();
        let info_width = (ui.available_width() * 0.26).clamp(205.0, 255.0);
        let cards_width = ui.available_width() - info_width - 8.0;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(cards_width, 94.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
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
                            latest.map_or("—".into(), |s| {
                                format!("{:.3} W", s.voltage * s.current)
                            }),
                            egui::Color32::from_rgb(108, 65, 167),
                        );
                    });
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(info_width, 94.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::Frame::new()
                        .fill(if ui.visuals().dark_mode {
                            egui::Color32::from_rgb(38, 47, 61)
                        } else {
                            egui::Color32::from_rgb(233, 239, 246)
                        })
                        .corner_radius(10.0)
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_min_width(info_width - 12.0);
                                ui.set_min_height(82.0);
                                ui.spacing_mut().item_spacing.y = 1.0;
                                let resistance = latest.and_then(|sample| {
                                    (sample.current > 0.001)
                                        .then(|| format!("{:.1}", sample.voltage / sample.current))
                                });
                                egui::Grid::new("secondary_readings")
                                    .num_columns(3)
                                    .spacing(egui::vec2(4.0, 1.0))
                                    .show(ui, |ui| {
                                        let dp_color =
                                            Metric::Dp.label_color(ui.visuals().dark_mode);
                                        Self::secondary_reading_row(
                                            ui,
                                            "D+",
                                            latest.map_or("—".into(), |s| format!("{:.4}", s.dp)),
                                            "V",
                                            dp_color,
                                            None,
                                        );
                                        let dm_color =
                                            Metric::Dm.label_color(ui.visuals().dark_mode);
                                        Self::secondary_reading_row(
                                            ui,
                                            "D−",
                                            latest.map_or("—".into(), |s| format!("{:.4}", s.dm)),
                                            "V",
                                            dm_color,
                                            None,
                                        );
                                        Self::secondary_reading_row(
                                            ui,
                                            lang.pick("负载电阻", "Load R"),
                                            resistance.unwrap_or_else(|| "—".into()),
                                            "Ω",
                                            Metric::Resistance.label_color(ui.visuals().dark_mode),
                                            None,
                                        );
                                        Self::secondary_reading_row(
                                            ui,
                                            lang.pick("纹波(参考)", "Ripple (ref.)"),
                                            self.ripple_vpp_mv()
                                                .map_or("—".into(), |value| format!("{value:.1}")),
                                            "mV",
                                            if ui.visuals().dark_mode {
                                                egui::Color32::from_rgb(61, 214, 192)
                                            } else {
                                                egui::Color32::from_rgb(0, 140, 129)
                                            },
                                            Some(lang.pick(
                                                "最近 1 秒电压采样的峰峰值。仪表最高只有 1000 SPS，充电器的高频开关纹波可能采不到，显示值可能与实际纹波相差较大。仅供参考。",
                                                "Peak-to-peak voltage from the latest 1 second of samples. The meter tops out at 1000 SPS, so high-frequency switching ripple from chargers may be missed and the reading may differ greatly from the actual ripple. For reference only.",
                                            )),
                                        );
                                    });
                            });
                        });
                },
            );
        });
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
        let dark_mode = ui.visuals().dark_mode;
        let plot_height = (ui.available_height() - 8.0).max(150.0);
        let mut plot = Plot::new("measurements_plot")
            .width(ui.available_width())
            .height(plot_height)
            .legend(Legend::default())
            .x_axis_label(lang.pick("秒", "Seconds"))
            .y_axis_label(lang.pick("数值", "Value"))
            .set_margin_fraction(egui::vec2(0.0, 0.05))
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show_crosshair(self.cursor_values);
        if self.cursor_values {
            let samples = &self.samples;
            let origin = self.chart_origin.unwrap_or(0.0);
            plot = plot.coordinates_formatter(
                Corner::LeftTop,
                CoordinatesFormatter::new(move |point, _| {
                    let Some(item) = closest_sample(samples, point.x + origin) else {
                        return String::new();
                    };
                    let mut text = format!("{:.3} s", item.seconds - origin);
                    for (index, metric) in Metric::ALL.iter().enumerate() {
                        if visible[index] {
                            if let Some(value) = metric.value(&item.sample) {
                                let decimals = match metric {
                                    Metric::Dp | Metric::Dm => 4,
                                    Metric::Resistance => 1,
                                    _ => 3,
                                };
                                text.push_str(&format!(
                                    "\n{}: {value:.decimals$}",
                                    metric.title(lang)
                                ));
                            }
                        }
                    }
                    text
                }),
            );
        }
        let end = self
            .samples
            .back()
            .map_or(0.1, |last| last.seconds - self.chart_origin.unwrap_or(0.0));
        let start = (end - self.window_secs as f64).max(0.0);
        let response = plot.show(ui, |plot| {
            plot.set_plot_bounds_x(start..=end.max(start + 0.1));
            for (index, metric) in Metric::ALL.iter().enumerate() {
                if visible[index] {
                    plot.line(
                        Line::new(metric.title(lang), self.chart_points(*metric))
                            .color(metric.label_color(dark_mode))
                            .width(2.0),
                    );
                }
            }
        });
        if self.cursor_values {
            response
                .response
                .on_hover_cursor(egui::CursorIcon::Crosshair);
        }
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

    fn protocol_panel(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.heading(lang.pick("协议检测", "Protocol scan"));
        ui.label(if self.scanning_protocols {
            lang.pick("正在逐项检测…", "Scanning protocols…")
        } else if self.protocol_results.is_empty() {
            lang.pick("等待检测", "Waiting for scan")
        } else {
            lang.pick("本次供电端的检测结果", "Results for this source")
        });
        egui::Frame::new()
            .fill(if ui.visuals().dark_mode {
                egui::Color32::from_rgb(90, 62, 24)
            } else {
                egui::Color32::from_rgb(255, 237, 192)
            })
            .corner_radius(6.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.label(lang.pick(
                    "⚠ 开始前请移除负载；检测会改变供电电压，带载检测可能损坏设备。",
                    "⚠ Remove the load first. Scanning changes the supply voltage and may damage a connected load.",
                ));
            });
        if ui
            .add_enabled(
                self.connected && !self.scanning_protocols,
                egui::Button::new(if self.protocol_results.is_empty() {
                    lang.pick("开始检测", "Start scan")
                } else {
                    lang.pick("重新检测", "Scan again")
                }),
            )
            .clicked()
        {
            let _ = self.commands.send(Command::ScanProtocols);
        }
        ui.separator();
        egui::Grid::new("protocol_tiles")
            .spacing([6.0, 6.0])
            .show(ui, |ui| {
                for (index, kind) in protocol::PROTOCOL_TYPES.iter().copied().enumerate() {
                    let result = self.protocol_results.get(&kind);
                    let supported = result.and_then(|item| item.supported);
                    let fill = match supported {
                        Some(true) => Self::protocol_color(kind),
                        Some(false) => egui::Color32::WHITE,
                        None if ui.visuals().dark_mode => egui::Color32::from_rgb(67, 77, 90),
                        None => egui::Color32::from_rgb(213, 221, 229),
                    };
                    let text_color = if supported == Some(true)
                        || (supported.is_none() && ui.visuals().dark_mode)
                    {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_rgb(46, 55, 63)
                    };
                    let state = match supported {
                        Some(true) => lang.pick("已发现", "Found"),
                        Some(false) => lang.pick("未发现", "Absent"),
                        None if self.scanning_protocols => lang.pick("待检测", "Pending"),
                        None => lang.pick("未上报", "No report"),
                    };
                    let label = format!("{}\n{}", protocol::protocol_name(kind), state);
                    let selected = self.selected_protocol == Some(kind);
                    let button =
                        egui::Button::new(egui::RichText::new(label).color(text_color).size(11.0))
                            .fill(fill)
                            .stroke(if selected {
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(27, 105, 186))
                            } else {
                                egui::Stroke::new(1.0, egui::Color32::from_gray(156))
                            });
                    if ui.add_sized([82.0, 50.0], button).clicked() {
                        self.selected_protocol = Some(kind);
                    }
                    if index % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        ui.separator();
        if let Some(kind) = self.selected_protocol {
            ui.heading(protocol::protocol_name(kind));
            if let Some(result) = self.protocol_results.get(&kind) {
                ui.label(match result.supported {
                    Some(true) => lang.pick("检测到支持", "Supported"),
                    Some(false) => lang.pick("未检测到支持", "Not detected"),
                    None => lang.pick("等待结果", "Waiting for result"),
                });
                if let Some(description) = &result.description {
                    ui.label(description.trim());
                }
                if let Some(mask) = result.mask {
                    ui.label(format!("Mask: 0x{mask:08X}"));
                }
            } else {
                ui.label(lang.pick("尚无结果", "No result yet"));
            }
        } else {
            ui.label(lang.pick(
                "点击协议方块查看详细档位",
                "Select a tile for details and voltage ranges",
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
        if self.protocol_dialog_open {
            egui::Panel::right("protocol_scan_panel")
                .resizable(false)
                .default_size(292.0)
                .min_size(292.0)
                .max_size(292.0)
                .show(ui, |ui| self.protocol_panel(ui));
        }
        egui::Panel::top("main_header")
            .resizable(false)
            .show_separator_line(false)
            .show(ui, |ui| self.header(ui));
        egui::Panel::bottom("resource_links")
            .resizable(false)
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .weak(),
                    );
                    ui.label("·");
                    ui.hyperlink_to(
                        self.language.pick("GitHub 仓库", "GitHub repository"),
                        "https://github.com/niu541412/ATK-UT70-RS",
                    );
                    ui.label("·");
                    ui.hyperlink_to(
                        self.language
                            .pick("UT70 官方资料", "Official UT70 resources"),
                        "https://www.openedv.com/docs/tool/USB/usb.html",
                    );
                    let hint = self.language.pick(
                        "负载电阻按 V/I 计算，它不是线阻。CSV 记录全部测量点。",
                        "Load resistance is V/I, not cable resistance. CSV stores all samples.",
                    );
                    let age = self.started_at.elapsed().as_secs_f32();
                    let opacity = ((5.0 - age) / 2.0).clamp(0.0, 1.0);
                    let response = ui.add(
                        egui::Label::new(
                            egui::RichText::new(hint)
                                .small()
                                .color(ui.visuals().weak_text_color().linear_multiply(opacity)),
                        )
                        .truncate(),
                    );
                    if opacity > 0.0 {
                        response.on_hover_text(hint);
                    }
                });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.plots(ui);
        });
        if self.settings_dialog_open {
            let mut open = true;
            egui::Window::new(self.language.pick("设置", "Settings"))
                .open(&mut open)
                .default_width(280.0)
                .show(ui.ctx(), |ui| {
                    let old_language = self.language;
                    egui::ComboBox::from_label(self.language.pick("语言", "Language"))
                        .selected_text(match self.language {
                            Language::Chinese => "简体中文",
                            Language::English => "English",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.language, Language::Chinese, "简体中文");
                            ui.selectable_value(&mut self.language, Language::English, "English");
                        });
                    let mut changed = self.language != old_language;
                    ui.horizontal(|ui| {
                        ui.label(self.language.pick("横轴时间范围", "Time range"));
                        egui::ComboBox::from_id_salt("time_window")
                            .selected_text(format!("{} s", self.window_secs))
                            .show_ui(ui, |ui| {
                                for seconds in settings::WINDOWS {
                                    changed |= ui
                                        .selectable_value(
                                            &mut self.window_secs,
                                            seconds,
                                            format!("{seconds} s"),
                                        )
                                        .changed();
                                }
                            });
                    });
                    changed |= ui
                        .checkbox(
                            &mut self.cursor_values,
                            self.language.pick("显示光标处数值", "Show cursor values"),
                        )
                        .changed();
                    if changed {
                        self.save_settings(ui.ctx());
                    }
                    ui.separator();
                    ui.label(format!("ATK UT70 RS v{}", env!("CARGO_PKG_VERSION")));
                    ui.label(self.language.pick(
                        "版权所有 © 2026 ATK-UT70-RS 贡献者",
                        "Copyright © 2026 ATK-UT70-RS contributors",
                    ));
                    ui.add(egui::Label::new(self.language.pick(
                        "本程序采用 GPL-3.0 许可证，允许再分发和修改；不提供任何担保。",
                        "Licensed under GPL-3.0. You may share and modify it. No warranty is provided.",
                    )).wrap());
                    ui.hyperlink_to(
                        self.language.pick("查看许可证", "View license"),
                        "https://github.com/niu541412/ATK-UT70-RS/blob/main/LICENSE",
                    );
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
        if let Some(screen) = self.pending_screen {
            let lang = self.language;
            let response = egui::Modal::new(egui::Id::unique("screen_switch_warning"))
                .show(ui.ctx(), |ui| {
                    if screen == 20 {
                        ui.heading(lang.pick("E-Marker 远程切换警告", "E-Marker switching warning"));
                        ui.label(lang.pick(
                            "远程切换 E-Marker（编号 20）曾使仪表立即显示 END 后白屏，也曾导致仪表自行重启。确认你可以手动恢复仪表后再继续。",
                            "Remote E-Marker switching (screen 20) has caused an immediate END screen followed by a blank display, and has also restarted the meter. Continue only if you can recover it manually.",
                        ));
                    } else {
                        ui.heading(lang.pick("自动检测警告", "Auto detection warning"));
                        ui.label(lang.pick(
                            "自动检测会与供电端握手并可能改变输出电压。请先移除负载，避免损坏连接的设备。",
                            "Auto detection negotiates with the power source and may change its output voltage. Remove the load first to avoid damaging connected devices.",
                        ));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(lang.pick("取消", "Cancel")).clicked() {
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.connected && !self.scanning_protocols,
                                egui::Button::new(if screen == 20 {
                                    lang.pick("确认发送", "Send anyway")
                                } else {
                                    lang.pick("已移除负载，继续", "Load removed, continue")
                                }),
                            )
                            .clicked()
                        {
                            let _ = self.commands.send(Command::SetScreen(screen));
                            ui.close();
                        }
                    });
                });
            if response.should_close() {
                self.pending_screen = None;
            }
        }
        ui.ctx()
            .request_repaint_after(if self.connected || self.flashing {
                Duration::from_millis(50)
            } else {
                Duration::from_millis(250)
            });
    }
}

fn lower_bound_time(samples: &VecDeque<TimedSample>, seconds: f64) -> usize {
    let (mut left, mut right) = (0, samples.len());
    while left < right {
        let middle = left + (right - left) / 2;
        if samples[middle].seconds < seconds {
            left = middle + 1;
        } else {
            right = middle;
        }
    }
    left
}

fn closest_sample(samples: &VecDeque<TimedSample>, seconds: f64) -> Option<&TimedSample> {
    let next = lower_bound_time(samples, seconds);
    match (next.checked_sub(1), samples.get(next)) {
        (Some(previous), Some(after)) => {
            let before = &samples[previous];
            if (seconds - before.seconds).abs() <= (after.seconds - seconds).abs() {
                Some(before)
            } else {
                Some(after)
            }
        }
        (Some(previous), None) => samples.get(previous),
        (None, after) => after,
    }
}

// 最近 1 秒电压采样的峰峰值，仅供参考，并非仪表模拟带宽内的完整纹波测量。
fn ripple_vpp_mv(samples: &VecDeque<TimedSample>) -> Option<f64> {
    let end = samples.back()?.seconds;
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    let mut count = 0;
    for item in samples
        .iter()
        .rev()
        .take_while(|item| item.seconds >= end - 1.0)
    {
        let voltage = item.sample.voltage as f64;
        if voltage.is_finite() {
            minimum = minimum.min(voltage);
            maximum = maximum.max(voltage);
            count += 1;
        }
    }
    (count >= 2).then_some((maximum - minimum) * 1000.0)
}

#[cfg(test)]
mod ripple_tests {
    use super::*;

    #[test]
    fn uses_only_the_latest_second_of_voltage_samples() {
        let mut samples = VecDeque::new();
        for (seconds, voltage) in [(0.0, 10.0), (1.0, 5.0), (1.5, 5.02), (2.0, 4.99)] {
            samples.push_back(TimedSample {
                seconds,
                sample: protocol::Sample {
                    voltage,
                    current: 0.0,
                    dp: 0.0,
                    dm: 0.0,
                },
            });
        }
        assert!((ripple_vpp_mv(&samples).unwrap() - 30.0).abs() < 0.01);
        assert_eq!(lower_bound_time(&samples, 1.25), 2);
        assert_eq!(closest_sample(&samples, 1.4).unwrap().seconds, 1.5);
        samples.truncate(1);
        assert_eq!(ripple_vpp_mv(&samples), None);
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
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    )
}
