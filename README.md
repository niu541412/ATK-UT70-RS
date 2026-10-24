# ATK-UT70-RS

正点原子 UT70 的跨平台上位机。启动时尝试连接仪表侧面的 USB HID 口，读取电压、电流、D+/D−，在同一图表中绘制最近 60 秒曲线；可选择显示功率及按 `V/I` 计算的负载电阻。支持调整采样率、记录完整 CSV，并通过独立的“固件更新”窗口读取本地官方 `.atk` 文件烧录固件。界面可切换简体中文和 English；使用 Rust、egui 和 egui_plot，HID 使用 hidapi。无需官方 Windows DLL。

此仓库包含[逆向后的 HID 接口文档](docs/PROTOCOL.md)和客户端源码，不包含官方程序、固件、USB 抓包或个人测试日志。协议来自一台硬件 2.7、应用 V3.5 的 UT70；其他版本还需实机验证。已知限制与验证状态见[文档](docs/PROTOCOL.md#验证边界)。

## 下载与运行

GitHub Actions 的 **Build desktop client** 工作流为以下架构生成独立产物，见每次构建的 Artifacts：

| 系统 | 架构 | 产物 |
| --- | --- | --- |
| Windows | x86_64、aarch64 | `.zip`，内含 `.exe` |
| macOS | x86_64、aarch64 | `.zip`，内含 `.app` |
| Linux | x86_64、aarch64 | `.AppImage` |

运行前连接 UT70 侧面 HID 口，关闭同时占用设备的官方客户端；软件启动时自动尝试连接，未连上时可点击“连接设备”重试。Linux 用户需要 HID 设备访问权限；常见发行版可创建 udev 规则：

```text
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="19f5", ATTRS{idProduct}=="6666", TAG+="uaccess"
```

规则生效后重新插拔 HID 线。AppImage 还需要系统提供图形驱动及 USB/HID 相关基础库。macOS 分发包目前没有 Apple 开发者签名或公证。

### 从源码构建

安装 Rust 稳定版和系统原生编译工具后：

```sh
cargo test --locked
cargo run --release
```

Linux 还需要 `libudev-dev`、`libxkbcommon-dev`、`libwayland-dev`、X11、GTK3 和 DBus 的开发包；具体安装命令见 [GitHub Actions 工作流](.github/workflows/build.yml)。Windows 需要 MSVC C++ Build Tools。

## 使用范围

- 测量：4、10、50、200、500、1000 SPS；电压、电流和功率以大字显示，六条可选曲线共用一个图表，以颜色及单位区分。负载电阻为 `V/I`，电流不高于 1 mA 时不绘制；它不是仪表“线阻模式”的线阻。每个收到的测量点写入 CSV，曲线降低绘图点数以控制占用。
- 协议检测：**尚未实现**。已确认的测量帧不含快充协议列表或档位；`0x20` 只有 DLL 静态线索，缺少自动检测流程和结果格式的可靠抓包。客户端目前不会主动触发可能改变被测端口输出的检测命令。
- 界面切换：只列出已有基础验证的编号；手动触发、自动检测、线阻模式、E-Marker 检测及“其他”暂不作为普通页面开放。立即回读编号不能证明模式长期稳定。
- 固件：先拔下 HID 线，按住仪表右键再插入，点击“连接设备”确认进入升级模式，再从“固件更新”窗口选择匹配当前硬件的官方 `.atk` 文件。客户端校验文件类型、长度、设备类型和硬件版本；每包检查回复，结束后确认设备重启及应用版本。固件文件必须由用户自行从官方渠道取得。升级过程中保持供电和 HID 连接。
- “波形显示”状态下版本查询曾超时，但测量帧仍继续。客户端连接时即使版本查询失败也会尝试接收测量；此时显示的采样率可能是默认值，直到再次成功查询或设置。

目前 Rust 客户端的协议单元测试与编译结果不能替代三平台实机验证。已有 Python 参考实现曾在 Windows、macOS、Linux 验证实时采集，并在 Windows、macOS 验证同版本烧录；Rust 移植后的烧录还需要独立实机验证。详见[升级协议](docs/FIRMWARE.md)。
