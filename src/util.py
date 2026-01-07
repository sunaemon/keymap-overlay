# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
import os
import re
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


def strip_c_comments(text: str) -> str:
    """
    Remove C-style // and /* */ comments from text.
    """
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    text = re.sub(r"//[^\n]*", "", text)
    return text
