#!/usr/bin/env python3
"""The emulator must receive a live peer door rather than a cached announcement."""
import importlib.util
from pathlib import Path
import unittest
import sys
import time
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location("mdns_door", Path(__file__).with_name("mdns-door.py"))
door = importlib.util.module_from_spec(spec)
spec.loader.exec_module(door)


class DoorTests(unittest.TestCase):
    def test_lines_in_one_write_are_all_read(self):
        command = [sys.executable, "-c", "import signal; print('first\\nsecond', flush=True); signal.pause()"]
        self.assertEqual(list(door.lines(command, time.monotonic() + 1)), ["first", "second"])

    def test_closed_cached_port_is_skipped(self):
        database = MagicMock()
        database.__enter__.return_value.execute.return_value.fetchone.return_value = ("peer",)
        records = [
            "peer.local.:1234", "id=peer kind=terminal name=Peer",
            "peer.local.:4567", "id=peer kind=terminal name=Peer",
        ]
        with patch.object(door.sqlite3, "connect", return_value=database), \
                patch.object(door, "lines", return_value=iter(records)), \
                patch.object(door.socket, "create_connection", side_effect=[ConnectionRefusedError(), MagicMock()]):
            self.assertEqual(door.resolve("peer.db")["door"], "10.0.2.2|4567|id=peer;kind=terminal;name=Peer")


if __name__ == "__main__":
    unittest.main()
