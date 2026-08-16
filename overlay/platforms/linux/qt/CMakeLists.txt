cmake_minimum_required(VERSION 3.21)

project(keymap-overlay-qt LANGUAGES CXX)

set(CMAKE_AUTOMOC ON)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 REQUIRED COMPONENTS Core DBus Gui Qml Quick)

qt_add_executable(keymap-overlay-qt main.cpp)

target_link_libraries(
  keymap-overlay-qt
  PRIVATE Qt6::Core Qt6::DBus Qt6::Gui Qt6::Qml Qt6::Quick
)

target_compile_options(
  keymap-overlay-qt
  PRIVATE $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall>
          $<$<CXX_COMPILER_ID:GNU,Clang>:-Wextra>
          $<$<CXX_COMPILER_ID:GNU,Clang>:-Wpedantic>
          $<$<CXX_COMPILER_ID:GNU,Clang>:-Werror>
)
