mod firmware;
mod protocol;
mod worker;

use eframe::egui;
use egui_plot::{Line, Plot};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use worker::{Command, Event, TimedSample};

struct App {
    commands: Sender<Command>,
    events: Receiver<Event>,
    samples: VecDeque<TimedSample>,
    status: String,
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
}

impl App {
    fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        std::thread::spawn(move || worker::run(command_rx, event_tx));
        Self {
            commands: command_tx,
            events: event_rx,
            samples: VecDeque::new(),
            status: "连接 UT70 侧面 HID 口后点击“连接”".into(),
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
        }
    }

    fn receive_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Status(message) => self.status = message,
                Event::Connected(version, rate) => {
                    self.connected = true;
                    self.bootloader = false;
                    self.rate = rate;
                    self.version = version;
                    self.status = format!("已连接；采样率 {rate} SPS");
                }
                Event::Bootloader(version) => {
                    self.connected = false;
                    self.bootloader = true;
                    self.version = Some(version);
                    self.status = "仪表处于升级模式，可选择本地 .atk 固件".into();
                }
                Event::Disconnected => self.connected = false,
                Event::Rate(rate) => {
                    self.rate = rate;
                    self.status = format!("采样率已设为 {rate} SPS");
                }
                Event::Screen(screen) => {
                    self.screen = screen;
                    self.status = format!("设备报告的界面编号：{screen}");
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
                Event::FlashDone(version) => {
                    self.flashing = false;
                    self.bootloader = false;
                    self.version = Some(version);
                    self.flash_confirmed = false;
                    self.status =
                        "烧录完成；设备已重启并通过应用版本校验。点击连接开始测量。".into();
                }
                Event::Error(message) => {
                    self.flashing = false;
                    self.status = message;
                }
            }
        }
    }

    fn latest(&self) -> Option<protocol::Sample> {
        self.samples.back().map(|item| item.sample)
    }

    fn chart_points(&self, pick: impl Fn(&protocol::Sample) -> f64) -> Vec<[f64; 2]> {
        // Keep at most about 2000 points per plot without changing the CSV stream.
        let width = self.samples.len().div_ceil(1000).max(1);
        let values: Vec<_> = self.samples.iter().collect();
        let mut out = Vec::with_capacity(values.len().min(2000));
        for bucket in values.chunks(width) {
            let mut low = bucket[0];
            let mut high = bucket[0];
            for &item in bucket {
                if pick(&item.sample) < pick(&low.sample) {
                    low = item;
                }
                if pick(&item.sample) > pick(&high.sample) {
                    high = item;
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
            out.push([first.seconds, pick(&first.sample)]);
            if first.seconds != second.seconds {
                out.push([second.seconds, pick(&second.sample)]);
            }
        }
        out
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.heading("ATK UT70");
        ui.label(&self.status);
        if let Some(v) = &self.version {
            ui.label(format!(
                "{} · 硬件 {}.{} · 应用 V{}.{} · Boot {} · 序列号 {}",
                if v.boot_mode {
                    "升级模式"
                } else {
                    "应用模式"
                },
                v.hardware / 10,
                v.hardware % 10,
                v.application / 10,
                v.application % 10,
                v.bootloader,
                v.serial
            ));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.connected && !self.flashing,
                    egui::Button::new("连接 / 检测模式"),
                )
                .clicked()
            {
                let _ = self.commands.send(Command::Connect);
                self.status = "正在连接…".into();
            }
            if ui
                .add_enabled(self.connected && !self.flashing, egui::Button::new("断开"))
                .clicked()
            {
                let _ = self.commands.send(Command::Disconnect);
            }
            if ui
                .add_enabled(
                    self.connected && !self.recording,
                    egui::Button::new("开始 CSV 记录"),
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
                .add_enabled(self.recording, egui::Button::new("停止记录"))
                .clicked()
            {
                let _ = self.commands.send(Command::StopCsv);
            }
        });
        ui.horizontal(|ui| {
            let mut selected_rate = self.rate;
            ui.add_enabled_ui(self.connected && !self.flashing && !self.recording, |ui| {
                egui::ComboBox::from_label("采样率")
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
                egui::ComboBox::from_label("仪表界面")
                    .selected_text(
                        protocol::SCREENS
                            .iter()
                            .find(|(id, _)| *id == self.screen)
                            .map(|(_, text)| *text)
                            .unwrap_or("未知"),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in protocol::SCREENS {
                            ui.selectable_value(&mut selected_screen, *id, *name);
                        }
                    });
            });
            if selected_screen != self.screen {
                let _ = self.commands.send(Command::SetScreen(selected_screen));
            }
        });
    }

    fn plots(&self, ui: &mut egui::Ui) {
        ui.separator();
        if let Some(s) = self.latest() {
            ui.horizontal(|ui| {
                ui.strong(format!("{:.4} V", s.voltage));
                ui.strong(format!("{:.4} A", s.current));
                ui.label(format!("{:.4} W", s.voltage * s.current));
                ui.label(format!("D+ {:.4} V  D− {:.4} V", s.dp, s.dm));
            });
        } else {
            ui.label("暂无测量数据");
        }
        Plot::new("voltage_plot")
            .height(220.0)
            .x_axis_label("秒")
            .y_axis_label("电压 V")
            .show(ui, |plot| {
                plot.line(Line::new("电压", self.chart_points(|s| s.voltage as f64)));
            });
        Plot::new("current_plot")
            .height(220.0)
            .x_axis_label("秒")
            .y_axis_label("电流 A")
            .show(ui, |plot| {
                plot.line(Line::new("电流", self.chart_points(|s| s.current as f64)));
            });
        ui.small("曲线显示最近 60 秒，并限制绘图点数；CSV 记录全部收到的测量点。");
    }

    fn flashing_controls(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("本地固件升级");
        ui.label("先拔下侧面 HID 线，按住仪表右键重新插入，使仪表进入升级模式。选择适配硬件版本的官方 .atk 文件。");
        if ui
            .add_enabled(!self.flashing, egui::Button::new("选择 .atk 文件"))
            .clicked()
        {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("ATK firmware", &["atk"])
                .pick_file()
            {
                match firmware::Firmware::read(&path) {
                    Ok(info) => {
                        self.firmware = Some((path, info));
                        self.flash_confirmed = false;
                    }
                    Err(e) => {
                        self.firmware = None;
                        self.status = format!("固件文件无效：{e}");
                    }
                }
            }
        }
        if let Some((path, info)) = &self.firmware {
            ui.label(path.display().to_string());
            ui.label(format!(
                "硬件 {}.{} · 目标应用 V{}.{} · 镜像 {} 字节 · 构建日期 {}",
                info.hardware / 10,
                info.hardware % 10,
                info.application / 10,
                info.application % 10,
                info.image_size,
                info.build_date
            ));
            ui.checkbox(
                &mut self.flash_confirmed,
                "确认设备已进入升级模式，烧录期间保持供电和 HID 连接",
            );
            if ui
                .add_enabled(
                    self.bootloader && self.flash_confirmed && !self.flashing,
                    egui::Button::new("开始烧录"),
                )
                .clicked()
            {
                let _ = self.commands.send(Command::Flash(path.clone()));
                self.flashing = true;
                self.progress = None;
                self.status = "正在准备升级…".into();
            }
        }
        if let Some((done, total)) = self.progress {
            ui.add(
                egui::ProgressBar::new(done as f32 / total as f32).text(format!("{done}/{total}")),
            );
        }
        if self.flashing {
            ui.label("烧录中，请保持程序运行和设备连接。");
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
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.header(ui);
                self.plots(ui);
                self.flashing_controls(ui);
            });
        });
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
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::new(App::new()))),
    )
}
