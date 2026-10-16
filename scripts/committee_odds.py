#!/usr/bin/env python3
"""Probability that an adversary holding a share f of all stake holds at least 342 of the
512 seats of one sync committee, per period and per year. Seats are sampled by effective
balance, so each seat is the adversary's with probability about f."""
from math import comb

SIZE, NEEDED = 512, 342
PERIODS_PER_YEAR = 365.25 * 24 * 3600 / (8192 * 12)


def capture(f: float) -> float:
    return sum(comb(SIZE, i) * f**i * (1 - f) ** (SIZE - i) for i in range(NEEDED, SIZE + 1))


if __name__ == "__main__":
    print(f"{'share':>6} {'per period':>12} {'per year':>12}")
    for f in (0.33, 0.40, 0.50, 0.55, 0.60, 0.65, 0.67):
        p = capture(f)
        print(f"{f:>6.0%} {p:>12.2e} {1 - (1 - p) ** PERIODS_PER_YEAR:>12.2e}")
