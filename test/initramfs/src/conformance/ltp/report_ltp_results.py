# SPDX-License-Identifier: MPL-2.0

"""Print abnormal LTP results and check their counters and exit codes."""

import json
import shlex
import sys

# LTP result bits: https://github.com/linux-test-project/ltp/blob/20260529/include/tst_res_flags.h
TFAIL = 1
TBROK = 2
TWARN = 4
TCONF = 32
LTP_EXIT_MASK = TFAIL | TBROK | TWARN | TCONF


def main():
    with open(sys.argv[1], encoding="utf-8") as report_file:
        report = json.load(report_file)

    if not report["results"]:
        raise ValueError("the LTP report contains no test results")

    stats = report["stats"]
    print(
        f"LTP report: failed={stats['failed']}, broken={stats['broken']}, "
        f"warnings={stats['warnings']}",
        flush=True,
    )
    failed = any(stats[key] for key in ("failed", "broken", "warnings"))

    for entry in report["results"]:
        test = entry["test"]
        return_code = int(test["retval"][0])
        abnormal_counters = any(test[key] for key in ("failed", "broken", "warnings"))
        if not abnormal_counters and return_code in (0, TCONF):
            continue

        failed = True
        name = entry["test_fqn"]
        print(f"\n=== LTP diagnostic: {name} ===")
        print("command:", shlex.join([test["command"], *test["arguments"]]))
        print(
            f"exit_code={return_code}, failed={test['failed']}, "
            f"broken={test['broken']}, warnings={test['warnings']}"
        )
        if (
            return_code >= 0
            and not return_code & ~LTP_EXIT_MASK
            and return_code & TWARN
            and not test["warnings"]
        ):
            print("Warning exit bit is set but the parsed warning count is zero.")

        log = test["log"]
        print("captured output:")
        print(log, end="" if log.endswith("\n") else "\n")
        print(f"=== End LTP diagnostic: {name} ===", flush=True)

    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
