import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import Pango from 'gi://Pango';
import St from 'gi://St';

import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BUS_NAME = 'com.sunaemon.KeymapOverlay';
const OBJECT_PATH = '/com/sunaemon/KeymapOverlay';
const RENDERER_INTERFACE = 'com.sunaemon.KeymapOverlay.Renderer1';
const INTERFACE_SCHEMA = 'org.gnome.desktop.interface';
const RENDERER_XML = `
<node>
  <interface name="${RENDERER_INTERFACE}">
    <method name="GetState">
      <arg type="t" name="generation" direction="out"/>
      <arg type="b" name="visible" direction="out"/>
      <arg type="s" name="model_json" direction="out"/>
    </method>
    <signal name="StateChanged">
      <arg type="t" name="generation"/>
      <arg type="b" name="visible"/>
      <arg type="s" name="model_json"/>
    </signal>
  </interface>
</node>`;
const RendererProxy = Gio.DBusProxy.makeProxyWrapper(RENDERER_XML);

export default class KeymapOverlayExtension extends Extension {
  enable() {
    this._enabled = true;
    this._generation = -1;
    this._overlay = null;
    this._interfaceSettings = new Gio.Settings({ schema_id: INTERFACE_SCHEMA });
    this._colorSchemeId = this._interfaceSettings.connect(
      'changed::color-scheme',
      () => this._syncColorScheme()
    );
    this._proxy = new RendererProxy(
      Gio.DBus.session,
      BUS_NAME,
      OBJECT_PATH,
      (proxy, error) => {
        if (!this._enabled) return;
        if (error) {
          console.error(`Keymap Overlay: ${error.message}`);
          return;
        }
        this._proxySignalId = proxy.connectSignal(
          'StateChanged',
          (_proxy, _sender, [generation, visible, modelJson]) =>
            this._applyState(generation, visible, modelJson)
        );
        this._ownerSignalId = proxy.connect('notify::g-name-owner', () => {
          this._generation = -1;
          if (proxy.g_name_owner) this._refreshState();
          else this._hide();
        });
        if (proxy.g_name_owner) this._refreshState();
      },
      null,
      Gio.DBusProxyFlags.DO_NOT_AUTO_START
    );
  }

  disable() {
    this._enabled = false;
    if (this._proxy && this._proxySignalId)
      this._proxy.disconnectSignal(this._proxySignalId);
    if (this._proxy && this._ownerSignalId)
      this._proxy.disconnect(this._ownerSignalId);
    this._proxy = null;
    this._proxySignalId = 0;
    this._ownerSignalId = 0;
    if (this._interfaceSettings && this._colorSchemeId)
      this._interfaceSettings.disconnect(this._colorSchemeId);
    this._interfaceSettings = null;
    this._colorSchemeId = 0;
    this._destroyOverlay();
  }

  _refreshState() {
    this._proxy.GetStateRemote((result, error) => {
      if (!this._enabled) return;
      if (error) {
        console.error(`Keymap Overlay: ${error.message}`);
        this._hide();
        return;
      }
      this._applyState(...result);
    });
  }

  _applyState(generation, visible, modelJson) {
    generation = Number(generation);
    if (generation <= this._generation) return;
    this._generation = generation;
    if (!visible) {
      this._hide();
      return;
    }

    let model;
    try {
      model = JSON.parse(modelJson);
      if (model.version !== 2 || model.width <= 0 || model.height <= 0)
        throw new Error('invalid model header');
    } catch (error) {
      console.error(`Keymap Overlay: cannot decode model: ${error.message}`);
      this._hide();
      return;
    }
    this._render(model);
  }

  _render(model) {
    this._destroyOverlay();
    const horizontalPadding = model.encoders.length
      ? Math.ceil(
          Math.max(...model.encoders.map((encoder) => encoder.size)) * 0.25
        )
      : 0;
    const overlay = new St.Widget({
      style_class: this._overlayStyleClass(),
      reactive: false,
      can_focus: false,
      clip_to_allocation: true,
      visible: false,
    });
    overlay.set_size(model.width + horizontalPadding * 2, model.height);
    const content = new St.Widget({ reactive: false, can_focus: false });
    content.set_position(horizontalPadding, 0);
    content.set_size(model.width, model.height);
    content.add_child(
      this._label(`L${model.layer}`, 20, 20, 60, model.header_font_size)
    );
    for (const key of model.keys) content.add_child(this._key(key, model));
    for (const encoder of model.encoders)
      content.add_child(this._encoder(encoder, model));
    overlay.add_child(content);

    Main.layoutManager.addTopChrome(overlay, {
      affectsStruts: false,
      trackFullscreen: true,
    });
    const monitor =
      Main.layoutManager.currentMonitor ?? Main.layoutManager.primaryMonitor;
    overlay.set_position(
      monitor.x + Math.round((monitor.width - overlay.width) / 2),
      monitor.y + Math.round((monitor.height - model.height) / 2)
    );
    overlay.show();
    this._overlay = overlay;
  }

  _key(key, model) {
    const actor = new St.Widget({
      style_class: 'button keymap-overlay-key',
      reactive: false,
      can_focus: false,
    });
    actor.set_position(key.x, key.y);
    actor.set_size(key.width, key.height);
    if (key.held) actor.add_style_pseudo_class('checked');
    actor.add_child(
      this._label(
        key.label.join('\n'),
        4,
        0,
        key.width - 8,
        model.key_font_size,
        key.height
      )
    );
    return actor;
  }

  _encoder(encoder, model) {
    const group = new St.Widget({ reactive: false, can_focus: false });
    group.set_position(encoder.x, encoder.y);
    group.set_size(encoder.size, encoder.size);
    const halfSize = encoder.size / 2;
    const labelGap = 3;
    const labelWidth = encoder.size * 0.75 - labelGap;

    const dial = new St.Widget({
      style_class: 'button keymap-overlay-encoder',
      reactive: false,
      can_focus: false,
    });
    dial.set_size(encoder.size, encoder.size);
    if (encoder.held) dial.add_style_pseudo_class('checked');
    dial.add_child(
      this._label(
        encoder.press ? `P ${encoder.press}` : '',
        4,
        0,
        encoder.size - 8,
        model.encoder_font_size,
        encoder.size
      )
    );
    group.add_child(dial);
    group.add_child(
      this._label(
        encoder.counter_clockwise.length
          ? `← ${this._compactEncoderActions(encoder.counter_clockwise)}`
          : '',
        halfSize - encoder.size * 0.75,
        -model.encoder_font_size * 2,
        labelWidth,
        model.encoder_font_size,
        model.encoder_font_size * 2,
        true,
        Pango.Alignment.RIGHT
      )
    );
    group.add_child(
      this._label(
        encoder.clockwise.length
          ? `${this._compactEncoderActions(encoder.clockwise)} →`
          : '',
        halfSize - labelGap,
        -model.encoder_font_size * 2,
        labelWidth,
        model.encoder_font_size,
        model.encoder_font_size * 2,
        true,
        Pango.Alignment.LEFT
      )
    );
    return group;
  }

  _compactEncoderActions(actions) {
    return actions
      .map((action) => action.replace(/^BRI\s*/, 'B').replace(/^VOL\s*/, 'V'))
      .join(' ');
  }

  _label(
    text,
    x,
    y,
    width,
    fontSize,
    height = fontSize * 2,
    singleLine = false,
    alignment = Pango.Alignment.CENTER
  ) {
    const textAlign =
      alignment === Pango.Alignment.LEFT
        ? 'left'
        : alignment === Pango.Alignment.RIGHT
          ? 'right'
          : 'center';
    const label = new St.Label({
      text,
      style_class: 'keymap-overlay-label',
      style: `font-size: ${fontSize}px; text-align: ${textAlign};`,
      x_align:
        alignment === Pango.Alignment.LEFT
          ? Clutter.ActorAlign.START
          : alignment === Pango.Alignment.RIGHT
            ? Clutter.ActorAlign.END
            : Clutter.ActorAlign.CENTER,
      y_align: Clutter.ActorAlign.CENTER,
    });
    label.clutter_text.set_line_wrap(!singleLine);
    label.clutter_text.set_line_alignment(alignment);
    label.clutter_text.set_ellipsize(
      singleLine ? Pango.EllipsizeMode.END : Pango.EllipsizeMode.NONE
    );
    const container = new St.Bin({
      child: label,
      reactive: false,
      can_focus: false,
      clip_to_allocation: true,
    });
    container.set_position(x, y);
    container.set_size(Math.max(1, width), Math.max(1, height));
    return container;
  }

  _hide() {
    this._destroyOverlay();
  }

  _syncColorScheme() {
    if (this._overlay)
      this._overlay.set_style_class_name(this._overlayStyleClass());
  }

  _overlayStyleClass() {
    const dark =
      this._interfaceSettings?.get_string('color-scheme') === 'prefer-dark';
    return dark
      ? 'popup-menu-content keymap-overlay keymap-overlay-dark'
      : 'keymap-overlay keymap-overlay-light';
  }

  _destroyOverlay() {
    if (!this._overlay) return;
    Main.layoutManager.removeChrome(this._overlay);
    this._overlay.destroy();
    this._overlay = null;
  }
}
