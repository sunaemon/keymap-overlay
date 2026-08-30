#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/input.h>
#include <linux/uhid.h>
#include <lzma.h>
#include <poll.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static const uint8_t report_descriptor[] = {
    0x06, 0x60, 0xff, // Usage Page (Vendor 0xff60)
    0x09, 0x61,       // Usage (0x61)
    0xa1, 0x01,       // Collection (Application)
    0x15, 0x00,       // Logical Minimum (0)
    0x26, 0xff, 0x00, // Logical Maximum (255)
    0x75, 0x08,       // Report Size (8)
    0x95, 0x20,       // Report Count (32)
    0x09, 0x62,       // Usage (0x62)
    0x81, 0x02,       // Input (Data, Variable, Absolute)
    0x09, 0x63,       // Usage (0x63)
    0x91, 0x02,       // Output (Data, Variable, Absolute)
    0xc0,             // End Collection
};

enum { report_size = 32, virtual_keyboard_id = 7 };

struct self_describing_device {
  uint8_t *definition;
  size_t definition_size;
  unsigned sequence_step;
};

static volatile sig_atomic_t running = 1;

static void stop(int signal_number) {
  (void)signal_number;
  running = 0;
}

static void install_signal_handlers(void) {
  const struct sigaction action = {.sa_handler = stop};
  if (sigaction(SIGINT, &action, NULL) < 0 ||
      sigaction(SIGTERM, &action, NULL) < 0) {
    perror("Failed to install signal handlers");
    exit(EXIT_FAILURE);
  }
}

static void write_event(int descriptor, const struct uhid_event *event) {
  const ssize_t length = write(descriptor, event, sizeof(*event));
  if (length < 0) {
    perror("Failed to write a UHID event");
    exit(EXIT_FAILURE);
  }
  if ((size_t)length != sizeof(*event)) {
    fprintf(stderr, "UHID accepted a short event\n");
    exit(EXIT_FAILURE);
  }
}

static void create_device(int descriptor) {
  struct uhid_event event = {0};
  event.type = UHID_CREATE2;
  snprintf((char *)event.u.create2.name, sizeof(event.u.create2.name),
           "Keymap Overlay E2E");
  snprintf((char *)event.u.create2.phys, sizeof(event.u.create2.phys),
           "uhid/keymap-overlay-e2e");
  snprintf((char *)event.u.create2.uniq, sizeof(event.u.create2.uniq),
           "keymap-overlay-e2e");
  event.u.create2.rd_size = sizeof(report_descriptor);
  event.u.create2.bus = BUS_USB;
  // Use a supported keyboard identity while the embedded definition retains a
  // distinct test-only KEYBOARD_ID. UHID has no USB parent attributes, so the
  // fixture-specific udev rule authorizes it by HID_NAME instead.
  event.u.create2.vendor = 0x355d;
  event.u.create2.product = 0x1001;
  memcpy(event.u.create2.rd_data, report_descriptor, sizeof(report_descriptor));
  write_event(descriptor, &event);
}

static void wait_until_opened(int descriptor) {
  struct uhid_event event = {0};
  while (running) {
    const ssize_t length = read(descriptor, &event, sizeof(event));
    if (length < 0 && errno == EINTR) {
      continue;
    }
    if (length < 0) {
      perror("Failed to read a UHID event");
      exit(EXIT_FAILURE);
    }
    if ((size_t)length < sizeof(event.type)) {
      fprintf(stderr, "UHID returned a short event\n");
      exit(EXIT_FAILURE);
    }
    if (event.type == UHID_OPEN) {
      return;
    }
  }
}

static void send_report(int descriptor, const uint8_t data[report_size]) {
  struct uhid_event event = {0};
  event.type = UHID_INPUT2;
  event.u.input2.size = report_size;
  memcpy(event.u.input2.data, data, report_size);
  write_event(descriptor, &event);
}

static void send_layer_event(int descriptor, uint8_t keyboard_id, uint8_t layer,
                             bool pressed) {
  uint8_t data[report_size] = {0};
  data[0] = 'K';
  data[1] = 'M';
  data[2] = 'O';
  data[3] = 1;
  data[4] = keyboard_id;
  data[5] = layer;
  data[6] = pressed ? 1 : 0;
  send_report(descriptor, data);
}

static uint8_t *read_definition(const char *path, size_t *size) {
  FILE *file = fopen(path, "rb");
  if (file == NULL) {
    perror("Failed to open the Vial definition");
    exit(EXIT_FAILURE);
  }
  if (fseek(file, 0, SEEK_END) != 0) {
    perror("Failed to seek the Vial definition");
    exit(EXIT_FAILURE);
  }
  const long length = ftell(file);
  if (length <= 0 || fseek(file, 0, SEEK_SET) != 0) {
    fprintf(stderr, "The Vial definition is empty or unreadable\n");
    exit(EXIT_FAILURE);
  }
  uint8_t *input = malloc((size_t)length);
  if (input == NULL ||
      fread(input, 1, (size_t)length, file) != (size_t)length) {
    fprintf(stderr, "Failed to read the Vial definition\n");
    exit(EXIT_FAILURE);
  }
  fclose(file);

  size_t capacity = lzma_stream_buffer_bound((size_t)length);
  uint8_t *compressed = malloc(capacity);
  size_t output_position = 0;
  if (compressed == NULL ||
      lzma_easy_buffer_encode(6, LZMA_CHECK_CRC64, NULL, input, (size_t)length,
                              compressed, &output_position,
                              capacity) != LZMA_OK) {
    fprintf(stderr, "Failed to compress the Vial definition\n");
    exit(EXIT_FAILURE);
  }
  free(input);
  *size = output_position;
  return compressed;
}

static const uint8_t *output_payload(const struct uhid_output_req *output) {
  if (output->size == report_size + 1 && output->data[0] == 0) {
    return &output->data[1];
  }
  if (output->size == report_size) {
    return output->data;
  }
  return NULL;
}

static void send_vial_response(int descriptor,
                               const struct self_describing_device *device,
                               const uint8_t request[report_size]) {
  static const uint8_t keymap[] = {
      0x00, 0x04, 0x52, 0x21, 0x00, 0x05, 0x00, 0x06, 0x00, 0x01, 0x00, 0x07,
      0x00, 0x01, 0x00, 0x08, 0x00, 0x01, 0x00, 0x09, 0x00, 0x01, 0x00, 0x0a,
  };
  uint8_t response[report_size] = {0};
  switch (request[0]) {
  case 0x01:
    response[0] = request[0];
    response[1] = 0x00;
    response[2] = 0x09;
    break;
  case 0x11:
    response[0] = request[0];
    response[1] = 3;
    break;
  case 0x12: {
    const size_t offset = ((size_t)request[1] << 8) | request[2];
    const size_t length = request[3];
    response[0] = request[0];
    response[1] = request[1];
    response[2] = request[2];
    response[3] = request[3];
    if (offset + length <= sizeof(keymap) && length <= report_size - 4) {
      memcpy(&response[4], &keymap[offset], length);
    } else {
      response[0] = 0xff;
    }
    break;
  }
  case 0xfe:
    switch (request[1]) {
    case 0x00:
      response[0] = request[0];
      response[1] = request[1];
      response[2] = virtual_keyboard_id;
      break;
    case 0x01:
      response[0] = (uint8_t)device->definition_size;
      response[1] = (uint8_t)(device->definition_size >> 8);
      response[2] = (uint8_t)(device->definition_size >> 16);
      response[3] = (uint8_t)(device->definition_size >> 24);
      break;
    case 0x02: {
      const size_t block = (size_t)request[2] | ((size_t)request[3] << 8) |
                           ((size_t)request[4] << 16) |
                           ((size_t)request[5] << 24);
      const size_t offset = block * report_size;
      if (offset < device->definition_size) {
        const size_t remaining = device->definition_size - offset;
        const size_t length = remaining < report_size ? remaining : report_size;
        memcpy(response, &device->definition[offset], length);
      }
      break;
    }
    default:
      response[0] = 0xff;
      break;
    }
    break;
  default:
    response[0] = 0xff;
    break;
  }
  send_report(descriptor, response);
}

static void run_self_describing(int descriptor,
                                struct self_describing_device *device) {
  while (running) {
    struct pollfd poll_descriptor = {.fd = descriptor, .events = POLLIN};
    const int result = poll(&poll_descriptor, 1, 2000);
    if (result < 0 && errno == EINTR) {
      continue;
    }
    if (result < 0) {
      perror("Failed to poll the UHID device");
      exit(EXIT_FAILURE);
    }
    if (result > 0) {
      struct uhid_event event = {0};
      const ssize_t length = read(descriptor, &event, sizeof(event));
      if (length < 0 && errno == EINTR) {
        continue;
      }
      if (length < 0) {
        perror("Failed to read a UHID request");
        exit(EXIT_FAILURE);
      }
      if (event.type == UHID_OUTPUT) {
        const uint8_t *request = output_payload(&event.u.output);
        if (request != NULL) {
          send_vial_response(descriptor, device, request);
        }
      }
      continue;
    }

    switch (device->sequence_step++ % 4) {
    case 0:
      send_layer_event(descriptor, virtual_keyboard_id, 1, true);
      break;
    case 1:
      send_layer_event(descriptor, virtual_keyboard_id, 2, true);
      break;
    case 2:
      send_layer_event(descriptor, virtual_keyboard_id, 2, false);
      break;
    default:
      send_layer_event(descriptor, virtual_keyboard_id, 1, false);
      break;
    }
  }
}

static void run_event_only(int descriptor) {
  while (running) {
    send_layer_event(descriptor, 1, 2, true);
    sleep(2);
    if (!running) {
      break;
    }
    send_layer_event(descriptor, 1, 2, false);
    sleep(1);
  }
}

int main(int argc, char **argv) {
  const bool self_describing =
      argc == 3 && strcmp(argv[1], "--definition") == 0;
  if (argc != 1 && !self_describing) {
    fprintf(stderr, "Usage: %s [--definition VIAL_JSON]\n", argv[0]);
    return EXIT_FAILURE;
  }

  struct self_describing_device device = {0};
  if (self_describing) {
    device.definition = read_definition(argv[2], &device.definition_size);
  }

  install_signal_handlers();

  const int descriptor = open("/dev/uhid", O_RDWR | O_CLOEXEC);
  if (descriptor < 0) {
    perror("Failed to open /dev/uhid");
    return EXIT_FAILURE;
  }
  create_device(descriptor);
  puts("Virtual Raw HID device ready");
  fflush(stdout);
  wait_until_opened(descriptor);

  if (self_describing) {
    run_self_describing(descriptor, &device);
  } else {
    run_event_only(descriptor);
  }
  free(device.definition);
  close(descriptor);
  return EXIT_SUCCESS;
}
