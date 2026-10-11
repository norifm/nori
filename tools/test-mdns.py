#!/usr/bin/env python3
"""The emulator must receive a live peer door rather than a cached announcement."""
import importlib.util
from pathlib import Path
import unittest
import sys
import time
from unittest.mock import MagicMock, patch
from types import SimpleNamespace

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
                patch.object(door.sys, "platform", "darwin"), \
                patch.object(door, "lines", return_value=iter(records)), \
                patch.object(door.socket, "create_connection", side_effect=[ConnectionRefusedError(), MagicMock()]):
            self.assertEqual(door.resolve("peer.db")["door"], "10.0.2.2|4567|id=peer;kind=terminal;name=Peer")

    def test_linux_discovery_skips_closed_ports(self):
        database = MagicMock()
        database.__enter__.return_value.execute.return_value.fetchone.return_value = ("peer",)
        zc = MagicMock()
        txt = {b"id": b"peer", b"kind": b"terminal", b"name": b"Linux Peer"}
        zc.get_service_info.side_effect = [SimpleNamespace(port=1234, properties=txt), SimpleNamespace(port=4567, properties=txt)]
        browser = MagicMock()

        def browse(instance, service_type, handlers):
            for _ in range(2):
                handlers[0](zeroconf=instance, service_type=service_type, name="nori-peer._nori._tcp.local.", state_change="added")
            return browser

        module = SimpleNamespace(Zeroconf=lambda: zc, ServiceBrowser=browse, ServiceStateChange=SimpleNamespace(Removed="removed"))
        with patch.object(door.sqlite3, "connect", return_value=database), \
                patch.object(door, "lines", return_value=iter([])), \
                patch.object(door.sys, "platform", "linux"), \
                patch.dict(sys.modules, {"zeroconf": module}), \
                patch.object(door.socket, "create_connection", side_effect=[ConnectionRefusedError(), MagicMock()]):
            self.assertEqual(door.resolve("peer.db")["door"], "10.0.2.2|4567|id=peer;kind=terminal;name=Linux Peer")
        browser.cancel.assert_called_once()
        zc.close.assert_called_once()


if __name__ == "__main__":
    unittest.main()
