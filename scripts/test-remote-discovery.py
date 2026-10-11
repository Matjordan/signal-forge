"""Exercise embedded remote discovery without modifying /dev or needing privileges."""
import contextlib
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

SOURCE = (Path(__file__).resolve().parents[1] / 'src/remote_serial.py').read_text()
class DiscoveryTests(unittest.TestCase):
    def discover(self, matches, targets, mode='discover-details'):
        output = io.StringIO()
        with patch('sys.argv', ['remote_serial.py', mode]), patch('glob.glob', side_effect=lambda pattern: matches.get(pattern, [])), patch('os.path.realpath', side_effect=lambda path: targets.get(path, path)), contextlib.redirect_stdout(output):
            with self.assertRaises(SystemExit) as stopped:
                exec(compile(SOURCE, 'remote_serial.py', 'exec'), {})
        self.assertEqual(stopped.exception.code, 0)
        return json.loads(output.getvalue())
    def test_by_id_precedes_tty_and_aliases_are_deduplicated(self):
        paths={'/dev/serial/by-id/*':['/dev/serial/by-id/b','/dev/serial/by-id/a'], '/dev/ttyUSB*':['/dev/ttyUSB0','/dev/ttyUSB1'], '/dev/ttyACM*':['/dev/ttyACM0']}
        targets={'/dev/serial/by-id/a':'/dev/ttyUSB0','/dev/serial/by-id/b':'/dev/ttyUSB0'}
        self.assertEqual(self.discover(paths,targets),[{'path':'/dev/serial/by-id/a','target':'/dev/ttyUSB0'},{'path':'/dev/ttyACM0','target':'/dev/ttyACM0'},{'path':'/dev/ttyUSB1','target':'/dev/ttyUSB1'}])
    def test_empty_and_legacy_path_response(self):
        self.assertEqual(self.discover({},{}),[])
        self.assertEqual(self.discover({'/dev/ttyS*':['/dev/ttyS0']},{},'discover'),['/dev/ttyS0'])
if __name__ == '__main__': unittest.main()
