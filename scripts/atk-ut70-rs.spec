%global debug_package %{nil}

Name: atk-ut70-rs
Version: %{app_version}
Release: 1
Summary: Desktop client for the ATK UT70 USB meter
License: GPL-3.0-only
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
install -Dm0644 %{_sourcedir}/LICENSE %{buildroot}%{_licensedir}/atk-ut70-rs/LICENSE
install -Dm0644 %{_sourcedir}/LICENSE.phosphor %{buildroot}%{_licensedir}/atk-ut70-rs/LICENSE.phosphor
install -Dm0644 %{_sourcedir}/OFL.Inter.txt %{buildroot}%{_licensedir}/atk-ut70-rs/OFL.Inter.txt

%files
%{_bindir}/atk-ut70-rs
%{_datadir}/applications/atk-ut70-rs.desktop
%{_datadir}/icons/hicolor/512x512/apps/atk-ut70-rs.png
%license %{_licensedir}/atk-ut70-rs/LICENSE
%license %{_licensedir}/atk-ut70-rs/LICENSE.phosphor
%license %{_licensedir}/atk-ut70-rs/OFL.Inter.txt
