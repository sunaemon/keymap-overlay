#include <QAccessible>
#include <QByteArray>
#include <QColor>
#include <QCursor>
#include <QDBusConnection>
#include <QDBusInterface>
#include <QDBusMessage>
#include <QDBusServiceWatcher>
#include <QDebug>
#include <QFont>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QObject>
#include <QPalette>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QScreen>
#include <QString>
#include <QTimer>
#include <QUrl>
#include <QVariant>
#include <QtGlobal>

#include <memory>
#include <stdexcept>
#include <string>

namespace {

constexpr auto BusName = "com.sunaemon.KeymapOverlay";
constexpr auto ObjectPath = "/com/sunaemon/KeymapOverlay";
constexpr auto RendererInterface = "com.sunaemon.KeymapOverlay.Renderer1";

bool is_gnome_desktop() {
  const auto desktop = qEnvironmentVariable("XDG_CURRENT_DESKTOP");
  for (const auto &part : desktop.split(':')) {
    if (part.compare(QStringLiteral("gnome"), Qt::CaseInsensitive) == 0) {
      return true;
    }
  }
  return false;
}

constexpr auto LayerShellImportMarker = "// LAYER_SHELL_IMPORT";
constexpr auto LayerShellPropertiesMarker = "    // LAYER_SHELL_PROPERTIES";

constexpr auto OverlayQml = R"QML(
import QtQuick
import QtQuick.Window
// LAYER_SHELL_IMPORT

Window {
    id: root
    property var overlayModel: ({ keys: [], encoders: [] })
    SystemPalette { id: systemPalette; colorGroup: SystemPalette.Active }

    visible: false
    color: "transparent"
    flags: Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
           | Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus
    width: overlayModel.width || 1
    height: overlayModel.height || 1

    // LAYER_SHELL_PROPERTIES

    Rectangle {
        id: panel
        objectName: "keymapOverlayPanel"
        anchors.fill: parent
        radius: 22
        color: Qt.rgba(systemPalette.window.r, systemPalette.window.g, systemPalette.window.b, 0.90)
        border.width: 1
        border.color: systemPalette.mid
        Accessible.role: Accessible.Grouping
        Accessible.name: "Keymap Overlay L" + (root.overlayModel.layer ?? "")
        Accessible.focusable: false
    }

    Text {
        x: 20
        y: 20
        text: "L" + (root.overlayModel.layer ?? "")
        color: systemPalette.windowText
        font.pixelSize: root.overlayModel.header_font_size || 14
        Accessible.role: Accessible.StaticText
        Accessible.name: text
        Accessible.focusable: false
    }

    Repeater {
        model: root.overlayModel.keys || []
        delegate: Rectangle {
            required property var modelData
            x: modelData.x
            y: modelData.y
            width: modelData.width
            height: modelData.height
            radius: 11
            color: modelData.held ? systemPalette.highlight : systemPalette.button
            border.width: 1
            border.color: systemPalette.mid

            Text {
                anchors.centerIn: parent
                width: parent.width - 8
                text: modelData.label.join("\n")
                color: modelData.held ? systemPalette.highlightedText : systemPalette.buttonText
                font.pixelSize: root.overlayModel.key_font_size || 10
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                wrapMode: Text.Wrap
                Accessible.role: Accessible.StaticText
                Accessible.name: text
                Accessible.focusable: false
            }
        }
    }

    Repeater {
        model: root.overlayModel.encoders || []
        delegate: Item {
            required property var modelData
            x: modelData.x
            y: modelData.y
            width: modelData.size
            height: modelData.size

            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: modelData.held ? systemPalette.highlight : systemPalette.button
                border.width: 1
                border.color: systemPalette.mid
            }

            Text {
                anchors.centerIn: parent
                text: modelData.press ? "P " + modelData.press : ""
                color: modelData.held ? systemPalette.highlightedText : systemPalette.buttonText
                font.pixelSize: root.overlayModel.encoder_font_size || 10
                Accessible.role: Accessible.StaticText
                Accessible.name: text
                Accessible.focusable: false
            }

            Text {
                anchors.right: parent.horizontalCenter
                anchors.rightMargin: 3
                anchors.bottom: parent.top
                width: Math.max(0, parent.width * 0.75 - 3)
                clip: true
                text: modelData.counter_clockwise.length
                      ? "← " + modelData.counter_clockwise.join(" ") : ""
                color: systemPalette.windowText
                font.pixelSize: root.overlayModel.encoder_font_size || 10
                horizontalAlignment: Text.AlignRight
                Accessible.role: Accessible.StaticText
                Accessible.name: text
                Accessible.focusable: false
            }

            Text {
                anchors.left: parent.horizontalCenter
                anchors.leftMargin: 3
                anchors.bottom: parent.top
                width: Math.max(0, parent.width * 0.75 - 3)
                clip: true
                text: modelData.clockwise.length
                      ? modelData.clockwise.join(" ") + " →" : ""
                color: systemPalette.windowText
                font.pixelSize: root.overlayModel.encoder_font_size || 10
                Accessible.role: Accessible.StaticText
                Accessible.name: text
                Accessible.focusable: false
            }
        }
    }
}
)QML";

QByteArray overlay_qml() {
  auto source = QByteArray(OverlayQml);
  if (QGuiApplication::platformName().startsWith(QStringLiteral("wayland"))) {
    source.replace(LayerShellImportMarker,
                   "import org.kde.layershell 1.0 as LayerShell");
    source.replace(
        LayerShellPropertiesMarker,
        "    LayerShell.Window.anchors: LayerShell.Window.AnchorNone\n"
        "    LayerShell.Window.exclusionZone: -1\n"
        "    LayerShell.Window.keyboardInteractivity: "
        "LayerShell.Window.KeyboardInteractivityNone\n"
        "    LayerShell.Window.layer: LayerShell.Window.LayerOverlay\n"
        "    LayerShell.Window.scope: \"keymap-overlay\"\n"
        "    LayerShell.Window.wantsToBeOnActiveScreen: true");
  } else {
    source.replace(LayerShellImportMarker, "");
    source.replace(LayerShellPropertiesMarker, "");
  }
  return source;
}

std::runtime_error qml_error(const QQmlComponent &component) {
  QStringList errors;
  for (const auto &error : component.errors()) {
    errors.append(error.toString());
  }
  return std::runtime_error(errors.join('\n').toStdString());
}

void apply_state(QQuickWindow &window, bool visible,
                 const QString &model_json) {
  if (!visible) {
    window.hide();
    return;
  }
  QJsonParseError parse_error;
  const auto document =
      QJsonDocument::fromJson(model_json.toUtf8(), &parse_error);
  if (parse_error.error != QJsonParseError::NoError || !document.isObject()) {
    throw std::runtime_error("Failed to parse an overlay model event: " +
                             parse_error.errorString().toStdString());
  }
  const auto model = document.object();
  const auto width = model.value(QStringLiteral("width")).toInt();
  const auto height = model.value(QStringLiteral("height")).toInt();
  if (model.value(QStringLiteral("version")).toInt() != 2 || width <= 0 ||
      height <= 0) {
    throw std::runtime_error("The overlay model event is invalid");
  }
  window.setProperty("overlayModel", model.toVariantMap());
  window.resize(width, height);
  if (QGuiApplication::platformName() == QStringLiteral("xcb")) {
    auto *screen = QGuiApplication::screenAt(QCursor::pos());
    if (!screen) {
      screen = QGuiApplication::primaryScreen();
    }
    if (screen) {
      const auto available = screen->availableGeometry();
      window.setPosition(available.x() + (available.width() - width) / 2,
                         available.y() + (available.height() - height) / 2);
    }
  }
  window.show();
}

void capture_golden_render(QQuickWindow &window) {
  const auto output = qEnvironmentVariable("KEYMAP_OVERLAY_GOLDEN_OUTPUT");
  if (output.isEmpty()) {
    return;
  }
  QTimer::singleShot(100, &window, [&window, output]() {
    if (!window.grabWindow().save(output)) {
      qCritical() << "Failed to save the golden render to" << output;
      QGuiApplication::exit(1);
    }
  });
}

void configure_golden_rendering(QGuiApplication &application) {
  if (!qEnvironmentVariableIsSet("KEYMAP_OVERLAY_GOLDEN_OUTPUT")) {
    return;
  }
  application.setFont(QFont(QStringLiteral("Noto Sans")));
  QPalette palette;
  palette.setColor(QPalette::Window, QColor(QStringLiteral("#f6f5f4")));
  palette.setColor(QPalette::WindowText, QColor(QStringLiteral("#2e3436")));
  palette.setColor(QPalette::Button, QColor(QStringLiteral("#deddda")));
  palette.setColor(QPalette::ButtonText, QColor(QStringLiteral("#2e3436")));
  palette.setColor(QPalette::Mid, QColor(QStringLiteral("#9a9996")));
  palette.setColor(QPalette::Highlight, QColor(QStringLiteral("#3584e4")));
  palette.setColor(QPalette::HighlightedText, Qt::white);
  application.setPalette(palette);
}

class RendererClient final : public QObject {
  Q_OBJECT

public:
  explicit RendererClient(QQuickWindow &window)
      : QObject(&window), window_(window),
        connection_(QDBusConnection::sessionBus()),
        owner_watcher_(QString::fromLatin1(BusName), connection_,
                       QDBusServiceWatcher::WatchForOwnerChange, this) {
    if (!connection_.isConnected()) {
      throw std::runtime_error("Failed to connect to the user D-Bus session");
    }
    QObject::connect(&owner_watcher_, &QDBusServiceWatcher::serviceOwnerChanged,
                     this, &RendererClient::service_owner_changed);
    if (!connection_.connect(QString::fromLatin1(BusName),
                             QString::fromLatin1(ObjectPath),
                             QString::fromLatin1(RendererInterface),
                             QStringLiteral("StateChanged"), this,
                             SLOT(state_changed(qulonglong, bool, QString)))) {
      throw std::runtime_error("Failed to subscribe to renderer state");
    }
    refresh_state();
  }

private slots:
  void state_changed(qulonglong generation, bool visible,
                     const QString &model_json) {
    try {
      apply_update(generation, visible, model_json);
    } catch (const std::exception &error) {
      fail(error);
    }
  }

private:
  void service_owner_changed(const QString &, const QString &,
                             const QString &new_owner) {
    generation_ = 0;
    if (new_owner.isEmpty()) {
      window_.hide();
      return;
    }
    try {
      refresh_state();
    } catch (const std::exception &error) {
      fail(error);
    }
  }

  void refresh_state() {
    QDBusInterface renderer(
        QString::fromLatin1(BusName), QString::fromLatin1(ObjectPath),
        QString::fromLatin1(RendererInterface), connection_);
    if (!renderer.isValid()) {
      throw std::runtime_error("The renderer D-Bus service is unavailable: " +
                               renderer.lastError().message().toStdString());
    }
    const auto reply = renderer.call(QStringLiteral("GetState"));
    if (reply.type() == QDBusMessage::ErrorMessage) {
      throw std::runtime_error("Failed to read renderer state: " +
                               reply.errorMessage().toStdString());
    }
    const auto arguments = reply.arguments();
    if (arguments.size() != 3) {
      throw std::runtime_error("The renderer D-Bus state has an invalid shape");
    }
    bool generation_ok = false;
    const auto generation = arguments.at(0).toULongLong(&generation_ok);
    if (!generation_ok || !arguments.at(1).canConvert<bool>() ||
        !arguments.at(2).canConvert<QString>()) {
      throw std::runtime_error("The renderer D-Bus state has invalid types");
    }
    apply_update(generation, arguments.at(1).toBool(),
                 arguments.at(2).toString());
  }

  void apply_update(qulonglong generation, bool visible,
                    const QString &model_json) {
    if (generation <= generation_) {
      return;
    }
    generation_ = generation;
    apply_state(window_, visible, model_json);
    if (visible && !captured_golden_render_) {
      captured_golden_render_ = true;
      capture_golden_render(window_);
    }
  }

  static void fail(const std::exception &error) {
    qCritical() << error.what();
    QGuiApplication::exit(1);
  }

  QQuickWindow &window_;
  QDBusConnection connection_;
  QDBusServiceWatcher owner_watcher_;
  qulonglong generation_ = 0;
  bool captured_golden_render_ = false;
};

} // namespace

int main(int argc, char *argv[]) {
  if (is_gnome_desktop() &&
      !qEnvironmentVariableIsSet("KEYMAP_OVERLAY_FORCE_QT")) {
    return 0;
  }

  QGuiApplication application(argc, argv);
  QGuiApplication::setApplicationName(QStringLiteral("keymap-overlay-qt"));
  QAccessible::setActive(true);
  configure_golden_rendering(application);

  try {
    QQmlEngine engine;
    QQmlComponent component(&engine);
    component.setData(overlay_qml(),
                      QUrl(QStringLiteral("qrc:/keymap-overlay.qml")));
    if (component.isError()) {
      throw qml_error(component);
    }
    std::unique_ptr<QObject> root(component.create());
    if (!root) {
      throw qml_error(component);
    }
    auto *window = qobject_cast<QQuickWindow *>(root.get());
    if (!window) {
      throw std::runtime_error("The Qt overlay root is not a window");
    }

    RendererClient renderer(*window);

    return application.exec();
  } catch (const std::exception &error) {
    qCritical() << error.what();
    return 1;
  }
}

#include "main.moc"
