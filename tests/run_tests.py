"""Run the same unittest discovery with failure details in GitHub annotations."""
import argparse
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--start', default=str(ROOT))
    args = parser.parse_args()
    # This is unittest's default discovery pattern; no filtering or skips.
    suite = unittest.defaultTestLoader.discover(args.start)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    for test, traceback in result.failures + result.errors:
        message = traceback.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print(f'::error title={test.id()}::{message}', flush=True)
    if result.testsRun == 0:
        print('::error::No tests executed', flush=True)
        return 1
    return 0 if result.wasSuccessful() else 1


if __name__ == '__main__':
    sys.exit(main())
