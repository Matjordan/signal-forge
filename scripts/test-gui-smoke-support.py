"""Regression checks for X11 startup races without needing a running X server."""
import subprocess
import unittest
from unittest.mock import Mock, patch

from gui_smoke_support import window_for


def result(stdout='', returncode=0, stderr=''):
    return subprocess.CompletedProcess([], returncode, stdout, stderr)


class WindowReadinessTests(unittest.TestCase):
    def setUp(self):
        self.process = Mock(pid=123, returncode=None)
        self.process.poll.return_value = None

    @patch('gui_smoke_support.time.sleep')
    @patch('gui_smoke_support.subprocess.run')
    def test_retries_badmatch_before_returning_a_focused_window(self, run, sleep):
        run.side_effect = [result('456\n'), result(returncode=1, stderr='BadMatch'),
                           result('456\n'), result(), result('456\n')]
        self.assertEqual(window_for(self.process), '456')
        self.assertIn('--onlyvisible', run.call_args_list[0].args[0])
        self.assertIn('--pid', run.call_args_list[0].args[0])
        self.assertIn('123', run.call_args_list[0].args[0])
        self.assertTrue(all('--sync' not in call.args[0] for call in run.call_args_list))
        sleep.assert_called_once()

    @patch('gui_smoke_support.time.sleep')
    @patch('gui_smoke_support.subprocess.run')
    def test_waits_for_confirmed_focus(self, run, sleep):
        run.side_effect = [result('456\n'), result(), result('1\n'),
                           result('456\n'), result(), result('456\n')]
        self.assertEqual(window_for(self.process), '456')
        sleep.assert_called_once()

    @patch('gui_smoke_support.time.sleep')
    @patch('gui_smoke_support.time.monotonic', side_effect=[0, 0, 11])
    @patch('gui_smoke_support.subprocess.run', return_value=result(returncode=1))
    def test_missing_visible_window_times_out(self, run, clock, sleep):
        with self.assertRaisesRegex(AssertionError, 'not ready within 10s'):
            window_for(self.process)

    @patch('gui_smoke_support.subprocess.run')
    def test_exited_application_fails_without_searching(self, run):
        self.process.poll.return_value = 1
        self.process.returncode = 1
        with self.assertRaisesRegex(AssertionError, 'Application exited: 1'):
            window_for(self.process)
        run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
