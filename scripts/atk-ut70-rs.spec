%global debug_package %{nil}

Name: atk-ut70-rs
Version: %{app_version}
Release: 1
Summary: Desktop client for the ATK UT70 USB meter
License: Unspecified
URL: https://github.com/niu541412/ATK-UT70-RS

%description
Read live measurements from the ATK UT70 USB HID interface and update firmware
using a local vendor firmware file.

%prep

%build

%install
install -Dm0755 %{_sourcedir}/atk-ut70-rs %{buildroot}%{_bindir}/atk-ut70-rs
install -Dm0644 %{_sourcedir}/atk-ut70-rs.desktop %{buildroot}%{_datadir}/applications/atk-ut70-rs.desktop
install -Dm0644 %{_sourcedir}/atk-ut70-rs.png %{buildroot}%{_datadir}/icons/hicolor/512x512/apps/atk-ut70-rs.png

%files
%{_bindir}/atk-ut70-rs
%{_datadir}/applications/atk-ut70-rs.desktop
%{_datadir}/icons/hicolor/512x512/apps/atk-ut70-rs.png
