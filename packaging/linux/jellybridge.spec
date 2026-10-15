# Built by scripts/package-linux.sh from the portable bundle; nothing is
# compiled here. Runtime dependencies come from rpmbuild's automatic soname
# scan, so the same package resolves on Fedora and openSUSE.
Name:           jellybridge
Version:        %{jb_version}
Release:        1
Summary:        Native desktop client for Jellyfin
License:        LicenseRef-Proprietary
URL:            https://github.com/oddbench-co/jellybridge
ExclusiveArch:  x86_64
AutoReqProv:    yes
# The bundled libmpv is private to the app: never offer it to the system and
# never ask the system for it.
%global __provides_exclude_from ^%{_libdir}/jellybridge/.*$|^/usr/lib/jellybridge/.*$
%global __requires_exclude ^(libmpv|libva|libva-drm)\\.so.*$
%global debug_package %{nil}
%global __strip /bin/true
# Loaded with dlopen by the windowing layer, so the soname scan cannot see them.
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libwayland-egl.so.1()(64bit)
Requires:       libX11.so.6()(64bit)
Requires:       libXcursor.so.1()(64bit)
Requires:       libXi.so.6()(64bit)
Requires:       libXrandr.so.2()(64bit)
Requires:       libGL.so.1()(64bit)
Requires:       hicolor-icon-theme

%description
JellyBridge browses and plays your Jellyfin library in a fast native window.
All common video codecs are included.

%install
cp -a %{jb_bundle}/usr %{buildroot}/

%files
/usr/bin/jellybridge
/usr/lib/jellybridge
/usr/share/applications/dev.nk.jellybridge.desktop
/usr/share/metainfo/dev.nk.jellybridge.metainfo.xml
/usr/share/icons/hicolor/*/apps/dev.nk.jellybridge.png
/usr/share/doc/jellybridge
