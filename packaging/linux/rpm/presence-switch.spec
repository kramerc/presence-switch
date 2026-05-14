Name:           presence-switch
# _version is passed via `rpmbuild --define "_version ..."` so the CI workflow
# and local build script can thread Cargo.toml's version through without
# requiring a manual edit here for each release.
Version:        %{_version}
# _release is overridden via `rpmbuild --define "_release ..."` for dev builds.
# When unset (e.g. a future tagged release), it falls back to `1`.
Release:        %{?_release}%{!?_release:1}%{?dist}
Summary:        Discord Rich Presence IPC proxy
License:        MIT
URL:            https://github.com/kramerc/presence-switch
Source0:        %{name}-%{version}.tar.gz
# Vendored crate dependencies — required because Fedora build environments
# (mock, koji, COPR) run offline.
Source1:        %{name}-%{version}-vendor.tar.xz

BuildRequires:  rust >= 1.85
BuildRequires:  cargo
BuildRequires:  systemd-rpm-macros
ExclusiveArch:  x86_64 aarch64

%description
A Discord Rich Presence IPC proxy that multiplexes RPC messages across
multiple running Discord instances. It binds the first available
discord-ipc-{0..9} socket name and relays incoming RPC frames to every
other Discord instance, so a single RPC client can broadcast its
presence to multiple Discord clients simultaneously.

Designed to run as a per-user systemd service; enable with:
  systemctl --user enable --now presence-switch

%prep
%autosetup -n %{name}-%{version}
tar -xf %{SOURCE1}
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

%build
cargo build --release --locked --offline

%install
install -Dm0755 target/release/presence-switch %{buildroot}%{_bindir}/presence-switch
install -Dm0644 packaging/linux/systemd/presence-switch.service \
    %{buildroot}%{_userunitdir}/presence-switch.service

%files
%license LICENSE
%doc README.md
%{_bindir}/presence-switch
%{_userunitdir}/presence-switch.service

%changelog
* Thu May 14 2026 Kramer Campbell <kramer@kramerc.com> - 0.1.0-1
- Initial RPM package
