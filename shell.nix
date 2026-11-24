{ pkgs ? import <nixpkgs> {} }:

let
  # WebKitGTK использует GStreamer для воспроизведения <video>.
  # Без этих плагинов WebKitWebProcess падает с
  # "GStreamer element autoaudiosink not found" и GLib-CRITICAL.
  gstPlugins = with pkgs.gst_all_1; [
    gstreamer
    gst-plugins-base
    gst-plugins-good
    gst-plugins-bad
    gst-plugins-ugly
    gst-libav
  ];
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    gobject-introspection
    cargo
    rustc
    nodejs
  ];

  buildInputs = with pkgs; [
    at-spi2-atk
    atkmm
    cairo
    gdk-pixbuf
    glib
    gtk3
    harfbuzz
    librsvg
    libsoup_3
    pango
    webkitgtk_4_1
    openssl
    dbus
    ffmpeg
    alsa-lib
  ] ++ gstPlugins;

  shellHook = ''
    # Пребилд .node-биндинги (например @tauri-apps/cli) не patchelf-нуты
    # и ищут liblzma.so.5 через LD_LIBRARY_PATH — добавляем xz явно,
    # чтобы не зависеть от состояния /etc/ld.so.cache.
    export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath (with pkgs; [
      gtk3
      webkitgtk_4_1
      glib
      cairo
      pango
      alsa-lib
      xz
    ] ++ gstPlugins)}:$LD_LIBRARY_PATH

    # Явно сообщаем WebKit/GStreamer, где искать плагины (autoaudiosink и др.).
    export GST_PLUGIN_SYSTEM_PATH=${pkgs.lib.makeSearchPath "lib/gstreamer-1.0" gstPlugins}:$GST_PLUGIN_SYSTEM_PATH
    export GST_PLUGIN_PATH=$GST_PLUGIN_SYSTEM_PATH

    # Аппаратные декодеры (NVDEC/VA-API) выдают кадры в memory:CUDAMemory,
    # которые WebKit не может нарисовать в <canvas> -> видео в предпросмотре пустое.
    # Принудительно используем программные декодеры (avdec_h264, vp8dec, vp9dec...).
    export GST_PLUGIN_FEATURE_RANK="nvh264dec:0,nvh265dec:0,nvvp8dec:0,nvvp9dec:0,vah264dec:0,vah265dec:0,vavp8dec:0,vavp9dec:0,vaapih264dec:0,vaapih265dec:0,vaapivp8dec:0,vaapivp9dec:0"
  '';
}
