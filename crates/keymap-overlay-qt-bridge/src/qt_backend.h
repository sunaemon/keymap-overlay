#pragma once

#include "rust/cxx.h"

#include <cstdint>

void run_qt_overlay(rust::Str assets_dir, std::int32_t event_fd);
