#include "src/qt_backend.h"

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QSocketNotifier>
#include <QString>
#include <QUrl>

#include <array>
#include <cerrno>
#include <cstdint>
#include <cstring>
#include <fcntl.h>
#include <map>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <sys/socket.h>
#include <system_error>
#include <unistd.h>

namespace {

constexpr std::uint8_t Show = 1;
constexpr std::uint8_t Hide = 2;

constexpr auto overlay_qml = R"QML(
import QtQuick
import QtQuick.Window
import org.kde.layershell 1.0 as LayerShell

Window {
    id: root
    property var overlayModel: ({ keys: [], encoders: [] })

    visible: false
    color: "transparent"
    flags: Qt.FramelessWindowHint | Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus
    width: overlayModel.width || 1
    height: overlayModel.height || 1

    LayerShell.Window.anchors: LayerShell.Window.AnchorNone
    LayerShell.Window.exclusionZone: -1
    LayerShell.Window.keyboardInteractivity: LayerShell.Window.KeyboardInteractivityNone
    LayerShell.Window.layer: LayerShell.Window.LayerOverlay
    LayerShell.Window.scope: "keymap-overlay"
    LayerShell.Window.wantsToBeOnActiveScreen: true

    Rectangle {
        anchors.fill: parent
        radius: 22
        color: "#E6F1F3F7"
        border.width: 1
        border.color: "#1F20242C"
    }

    Text {
        x: 20
        y: 20
        text: "L" + (root.overlayModel.layer ?? "")
        color: "#20242C"
        font.pixelSize: root.overlayModel.header_font_size || 14
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
            color: modelData.held ? "#FFFFDDDD" : "#F6DCE0E7"
            border.width: 1
            border.color: "#1F20242C"

            Text {
                anchors.centerIn: parent
                width: parent.width - 8
                text: modelData.label.join("\n")
                color: "#20242C"
                font.pixelSize: root.overlayModel.key_font_size || 10
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                wrapMode: Text.Wrap
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
                color: modelData.held ? "#FFFFDDDD" : "#F6DCE0E7"
                border.width: 1
                border.color: "#1F20242C"
            }

            Text {
                anchors.centerIn: parent
                text: modelData.press ? "P " + modelData.press : ""
                color: "#20242C"
                font.pixelSize: root.overlayModel.encoder_font_size || 10
            }

            Text {
                anchors.right: parent.horizontalCenter
                anchors.rightMargin: 3
                anchors.bottom: parent.top
                text: modelData.counter_clockwise.length
                      ? "← " + modelData.counter_clockwise.join(" ") : ""
                color: "#20242C"
                font.pixelSize: root.overlayModel.encoder_font_size || 10
                horizontalAlignment: Text.AlignRight
            }

            Text {
                anchors.left: parent.horizontalCenter
                anchors.leftMargin: 3
                anchors.bottom: parent.top
                text: modelData.clockwise.length
                      ? modelData.clockwise.join(" ") + " →" : ""
                color: "#20242C"
                font.pixelSize: root.overlayModel.encoder_font_size || 10
            }
        }
    }
}
)QML";

class OwnedFileDescriptor {
public:
  explicit OwnedFileDescriptor(int descriptor) : descriptor_(descriptor) {}
  ~OwnedFileDescriptor() {
    if (descriptor_ >= 0) {
      ::close(descriptor_);
    }
  }
  OwnedFileDescriptor(const OwnedFileDescriptor &) = delete;
  OwnedFileDescriptor &operator=(const OwnedFileDescriptor &) = delete;
  int get() const { return descriptor_; }

private:
  int descriptor_;
};

using ModelKey = std::pair<std::uint8_t, std::uint8_t>;
using Models = std::map<ModelKey, QJsonObject>;

std::runtime_error qml_error(const QQmlComponent &component) {
  QStringList errors;
  for (const auto &error : component.errors()) {
    errors.append(error.toString());
  }
  return std::runtime_error(errors.join('\n').toStdString());
}

Models load_models(const QString &assets_dir) {
  const QDir directory(assets_dir);
  if (!directory.exists()) {
    throw std::runtime_error("Overlay asset directory does not exist: " +
                             assets_dir.toStdString());
  }

  const QRegularExpression filename_pattern(
      QStringLiteral(R"(^(\d+)_L(\d+)\.json$)"));
  Models models;
  for (const auto &filename :
       directory.entryList({QStringLiteral("*_L*.json")}, QDir::Files)) {
    const auto match = filename_pattern.match(filename);
    if (!match.hasMatch()) {
      continue;
    }
    bool keyboard_ok = false;
    bool layer_ok = false;
    const auto keyboard_id = match.captured(1).toUInt(&keyboard_ok);
    const auto layer = match.captured(2).toUInt(&layer_ok);
    if (!keyboard_ok || !layer_ok || keyboard_id > 255 || layer > 255) {
      continue;
    }

    QFile file(directory.filePath(filename));
    if (!file.open(QIODevice::ReadOnly)) {
      throw std::runtime_error("Failed to open overlay model: " +
                               file.fileName().toStdString());
    }
    QJsonParseError parse_error;
    const auto document = QJsonDocument::fromJson(file.readAll(), &parse_error);
    if (parse_error.error != QJsonParseError::NoError || !document.isObject()) {
      throw std::runtime_error("Failed to parse overlay model " +
                               file.fileName().toStdString() + ": " +
                               parse_error.errorString().toStdString());
    }
    const auto model = document.object();
    if (model.value(QStringLiteral("version")).toInt() != 1) {
      throw std::runtime_error("Unsupported overlay model version in " +
                               file.fileName().toStdString());
    }
    if (model.value(QStringLiteral("layer")).toInt(-1) !=
        static_cast<int>(layer)) {
      throw std::runtime_error(
          "Overlay model layer does not match its filename: " +
          file.fileName().toStdString());
    }
    models.emplace(ModelKey{static_cast<std::uint8_t>(keyboard_id),
                            static_cast<std::uint8_t>(layer)},
                   model);
  }
  return models;
}

void apply_packet(QQuickWindow &window, const Models &models,
                  const std::array<std::uint8_t, 3> &packet) {
  if (packet[0] == Hide) {
    window.hide();
    return;
  }
  if (packet[0] != Show) {
    return;
  }
  const auto model = models.find({packet[1], packet[2]});
  if (model == models.end()) {
    qWarning() << "Overlay model is unavailable for keyboard"
               << static_cast<int>(packet[1]) << "layer"
               << static_cast<int>(packet[2]);
    window.hide();
    return;
  }
  window.setProperty("overlayModel", model->second.toVariantMap());
  window.resize(model->second.value(QStringLiteral("width")).toInt(),
                model->second.value(QStringLiteral("height")).toInt());
  window.show();
}

void drain_packets(OwnedFileDescriptor &descriptor, QQuickWindow &window,
                   const Models &models) {
  std::array<std::uint8_t, 3> packet{};
  std::optional<std::array<std::uint8_t, 3>> latest;
  while (true) {
    const auto count =
        ::recv(descriptor.get(), packet.data(), packet.size(), 0);
    if (count == static_cast<ssize_t>(packet.size())) {
      latest = packet;
      continue;
    }
    if (count == -1 && errno == EINTR) {
      continue;
    }
    if (count == -1 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      break;
    }
    if (count == 0) {
      QGuiApplication::quit();
      return;
    }
    if (count < 0) {
      throw std::system_error(errno, std::generic_category(),
                              "Failed to receive Qt events");
    }
    throw std::runtime_error(
        "The Qt event socket returned a malformed datagram");
  }
  if (latest) {
    apply_packet(window, models, *latest);
  }
}

} // namespace

void run_qt_overlay(rust::Str assets_dir, std::int32_t event_fd) {
  OwnedFileDescriptor descriptor(event_fd);
  int argc = 1;
  char program_name[] = "keymap-overlay";
  char *argv[] = {program_name, nullptr};
  QGuiApplication application(argc, argv);

  const auto path = QString::fromUtf8(
      assets_dir.data(), static_cast<qsizetype>(assets_dir.size()));
  const auto models = load_models(path);
  QQmlEngine engine;
  QQmlComponent component(&engine);
  component.setData(overlay_qml,
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

  const auto old_flags = ::fcntl(descriptor.get(), F_GETFL);
  if (old_flags < 0 ||
      ::fcntl(descriptor.get(), F_SETFL, old_flags | O_NONBLOCK) < 0) {
    throw std::system_error(errno, std::generic_category(),
                            "Failed to make the Qt event socket non-blocking");
  }
  QSocketNotifier notifier(descriptor.get(), QSocketNotifier::Read);
  QObject::connect(&notifier, &QSocketNotifier::activated,
                   [&descriptor, window, &models] {
                     try {
                       drain_packets(descriptor, *window, models);
                     } catch (const std::exception &error) {
                       qCritical() << error.what();
                       QGuiApplication::exit(1);
                     }
                   });

  const auto result = application.exec();
  if (result != 0) {
    throw std::runtime_error("The Qt event loop exited with status " +
                             std::to_string(result));
  }
}
