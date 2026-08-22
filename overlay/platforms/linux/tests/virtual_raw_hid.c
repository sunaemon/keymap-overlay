#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/input.h>
#include <linux/uhid.h>
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
    0xc0,             // End Collection
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
  event.u.create2.vendor = 0x1209;
  event.u.create2.product = 0x4b4d;
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

static void send_layer_event(int descriptor, bool pressed) {
  struct uhid_event event = {0};
  event.type = UHID_INPUT2;
  event.u.input2.size = 32;
  event.u.input2.data[0] = 'K';
  event.u.input2.data[1] = 'M';
  event.u.input2.data[2] = 'O';
  event.u.input2.data[3] = 1;
  event.u.input2.data[4] = 1;
  event.u.input2.data[5] = 2;
  event.u.input2.data[6] = pressed ? 1 : 0;
  write_event(descriptor, &event);
}

int main(void) {
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

  while (running) {
    send_layer_event(descriptor, true);
    sleep(2);
    if (!running) {
      break;
    }
    send_layer_event(descriptor, false);
    sleep(1);
  }
  close(descriptor);
  return EXIT_SUCCESS;
}
