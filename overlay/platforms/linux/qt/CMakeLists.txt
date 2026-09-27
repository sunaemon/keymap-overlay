cmake_minimum_required(VERSION 3.21)

project(keymap-overlay-qt LANGUAGES CXX)

file(READ "${CMAKE_CURRENT_SOURCE_DIR}/../../../../Cargo.toml" WORKSPACE_CARGO_TOML)
string(REGEX MATCH "version = \"([^\"]+)\"" _ "${WORKSPACE_CARGO_TOML}")
set(KEYMAP_OVERLAY_VERSION "${CMAKE_MATCH_1}")

set(CMAKE_AUTOMOC ON)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 REQUIRED COMPONENTS Core DBus Gui Qml Quick Widgets)

qt_add_executable(keymap-overlay-qt main.cpp)
target_compile_definitions(
  keymap-overlay-qt PRIVATE KEYMAP_OVERLAY_VERSION="${KEYMAP_OVERLAY_VERSION}"
)

target_link_libraries(
  keymap-overlay-qt
  PRIVATE Qt6::Core Qt6::DBus Qt6::Gui Qt6::Qml Qt6::Quick Qt6::Widgets
)

target_compile_options(
  keymap-overlay-qt
  PRIVATE $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall>
          $<$<CXX_COMPILER_ID:GNU,Clang>:-Wextra>
          $<$<CXX_COMPILER_ID:GNU,Clang>:-Wpedantic>
          $<$<CXX_COMPILER_ID:GNU,Clang>:-Werror>
)
