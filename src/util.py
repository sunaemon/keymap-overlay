# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
import os
import sys

# Configure logging to stderr so it doesn't interfere with stdout redirection
log_level = os.environ.get("LOG_LEVEL", "INFO").upper()
logging.basicConfig(
    level=getattr(logging, log_level, logging.INFO),
    format="%(levelname)s: %(message)s",
    stream=sys.stderr,
)


def get_logger(name: str) -> logging.Logger:
    """
    Returns a logger instance with the given name.
    """
    return logging.getLogger(name)
