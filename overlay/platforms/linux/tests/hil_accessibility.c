#define _GNU_SOURCE

// Copyright 2026 sunaemon
// SPDX-License-Identifier: MIT

#include <atspi/atspi.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct probe_result {
  const char *expected_label;
  bool require_overlay;
  bool found_overlay;
  bool found_label;
  bool overlay_focused;
  char *focused_identity;
};

static void fail_error(const char *operation, GError *error) {
  fprintf(stderr, "Linux accessibility HIL failure: %s: %s\n", operation,
          error == NULL ? "unknown AT-SPI error" : error->message);
  if (error != NULL) {
    g_error_free(error);
  }
  exit(EXIT_FAILURE);
}

static char *accessible_name(AtspiAccessible *accessible) {
  GError *error = NULL;
  char *name = atspi_accessible_get_name(accessible, &error);
  if (error != NULL) {
    fail_error("read accessible name", error);
  }
  return name;
}

static bool has_state(AtspiAccessible *accessible, AtspiStateType state) {
  AtspiStateSet *states = atspi_accessible_get_state_set(accessible);
  if (states == NULL) {
    return false;
  }
  const bool contains = atspi_state_set_contains(states, state);
  g_object_unref(states);
  return contains;
}

static void inspect_accessible(AtspiAccessible *accessible, bool in_overlay,
                               struct probe_result *result, unsigned depth) {
  if (depth > 64) {
    return;
  }

  char *name = accessible_name(accessible);
  const bool is_overlay =
      in_overlay || (name != NULL && (strcmp(name, "keymap-overlay-qt") == 0 ||
                                      strstr(name, "Keymap Overlay") != NULL));
  if (is_overlay) {
    result->found_overlay = true;
    result->overlay_focused |= has_state(accessible, ATSPI_STATE_FOCUSED);
  }
  if (is_overlay && name != NULL && strcmp(name, result->expected_label) == 0) {
    result->found_label = true;
  }
  if (has_state(accessible, ATSPI_STATE_FOCUSED)) {
    free(result->focused_identity);
    const AtspiRole role = atspi_accessible_get_role(accessible, NULL);
    if (asprintf(&result->focused_identity, "%u:%s", (unsigned)role,
                 name == NULL ? "" : name) < 0) {
      fprintf(stderr, "Linux accessibility HIL failure: out of memory\n");
      exit(EXIT_FAILURE);
    }
  }

  GError *error = NULL;
  const int child_count = atspi_accessible_get_child_count(accessible, &error);
  if (error != NULL) {
    fail_error("read accessible child count", error);
  }
  for (int index = 0; index < child_count; ++index) {
    AtspiAccessible *child =
        atspi_accessible_get_child_at_index(accessible, index, &error);
    if (error != NULL) {
      fail_error("read accessible child", error);
    }
    if (child != NULL) {
      inspect_accessible(child, is_overlay, result, depth + 1);
      g_object_unref(child);
    }
  }
  g_free(name);
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "Usage: %s EXPECTED_LABEL\n", argv[0]);
    return EXIT_FAILURE;
  }
  if (atspi_init() != 0) {
    fprintf(stderr,
            "Linux accessibility HIL failure: AT-SPI initialization failed\n");
    return EXIT_FAILURE;
  }

  struct probe_result result = {.expected_label = argv[1],
                                .require_overlay = strcmp(argv[1], "-") != 0};
  const int desktop_count = atspi_get_desktop_count();
  for (int index = 0; index < desktop_count; ++index) {
    AtspiAccessible *desktop = atspi_get_desktop(index);
    if (desktop != NULL) {
      inspect_accessible(desktop, false, &result, 0);
      g_object_unref(desktop);
    }
  }
  atspi_exit();

  if (!result.require_overlay) {
    printf("FOCUSED=%s\n",
           result.focused_identity == NULL ? "none" : result.focused_identity);
    free(result.focused_identity);
    return EXIT_SUCCESS;
  }
  if (!result.found_overlay) {
    fprintf(stderr,
            "Linux accessibility HIL failure: overlay is absent from AT-SPI\n");
    return EXIT_FAILURE;
  }
  if (!result.found_label) {
    fprintf(stderr,
            "Linux accessibility HIL failure: expected label '%s' is absent\n",
            result.expected_label);
    return EXIT_FAILURE;
  }
  if (result.overlay_focused) {
    fprintf(stderr,
            "Linux accessibility HIL failure: an overlay element is focused\n");
    return EXIT_FAILURE;
  }
  printf("PASS: label=%s focused=%s overlay-focused=false\n",
         result.expected_label,
         result.focused_identity == NULL ? "none" : result.focused_identity);
  free(result.focused_identity);
  return EXIT_SUCCESS;
}
