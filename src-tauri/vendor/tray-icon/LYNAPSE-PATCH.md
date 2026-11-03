Unmodified tray-icon 0.25.1 (MIT/Apache-2.0) except src/platform_impl/gtk/mod.rs: set_title() / attrs.title set the
AppIndicator *title* (StatusNotifierItem Title = KDE hover text) instead of the panel label. Tooltips are a no-op in
upstream on Linux, so this is the only way to get hover text on the tray icon.
