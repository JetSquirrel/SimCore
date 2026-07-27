# SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Faithful SimPy port of the hospital model in examples/compare.rs.

Mirrors the same parameters and the same statistical-comparison policy:
  * nurse  -> simpy.PriorityResource(1), request(priority=triage)
  * beds   -> simpy.Resource(3) plus a manual eviction map: a critical patient
              with all beds full fires the longest-admitted patient's event,
              which loses the treatment-vs-eviction race and frees the bed.
  * blood  -> simpy.Container(capacity=100, init=60)
  * ed_cleared_at = latest patient discharge time
  * no forced-completion deadline: patients blocked on a depleted blood bank
    simply never discharge; the run ends when the event queue drains.

Ignores --n/--lambda/--mu (the model carries its own constants).
"""

import simpy

from _common import run
from _feed import SplitMix64

SIM_DURATION = 480.0
ARRIVAL_SCALE = 8.0      # mean inter-arrival (1 / rate)
TRIAGE_DURATION = 5.0
MEAN_TREATMENT = 20.0
CRITICAL_PROB = 0.3
BLOOD_CAPACITY = 100.0
BLOOD_INITIAL = 60.0
BLOOD_RESTOCK = 20.0
RESTOCK_INTERVAL = 60.0
BLOOD_CRITICAL = 10.0
BLOOD_STANDARD = 2.0


def run_seed(seed, args):
    rng = SplitMix64(seed)
    env = simpy.Environment()
    nurse = simpy.PriorityResource(env, capacity=1)
    beds = simpy.Resource(env, capacity=3)
    blood = simpy.Container(env, capacity=BLOOD_CAPACITY, init=BLOOD_INITIAL)
    evict = {}  # patient_id -> simpy.Event (longest-admitted = smallest id)

    stats = {
        "critical_treated": 0, "standard_treated": 0, "early_discharged": 0,
        "blood_bank_waits": 0, "total_nurse_wait": 0.0, "total_bed_wait": 0.0,
        "total_blood_wait": 0.0, "ed_cleared_at": 0.0,
    }

    def restock():
        while True:
            yield env.timeout(RESTOCK_INTERVAL)
            if env.now > SIM_DURATION:
                break
            yield blood.put(BLOOD_RESTOCK)

    def patient(pid, triage, treatment):
        blood_units = BLOOD_CRITICAL if triage == 0 else BLOOD_STANDARD

        nstart = env.now
        with nurse.request(priority=triage) as nreq:
            yield nreq
            nurse_wait = env.now - nstart
            yield env.timeout(TRIAGE_DURATION)

        bstart = env.now
        yield blood.get(blood_units)
        blood_wait = env.now - bstart
        if blood_wait > 0:
            stats["blood_bank_waits"] += 1

        # Critical patient evicts the longest-admitted patient if beds are full.
        if triage == 0 and beds.count >= beds.capacity and evict:
            vid = min(evict)
            ev = evict.pop(vid)
            if not ev.triggered:
                ev.succeed()

        bedstart = env.now
        bedreq = beds.request()
        yield bedreq
        bed_wait = env.now - bedstart
        admitted_at = env.now

        my_ev = env.event()
        evict[pid] = my_ev
        yield env.timeout(treatment) | my_ev
        was_early = env.now < admitted_at + treatment
        evict.pop(pid, None)
        beds.release(bedreq)

        if triage == 0:
            stats["critical_treated"] += 1
        else:
            stats["standard_treated"] += 1
        if was_early:
            stats["early_discharged"] += 1
        stats["total_nurse_wait"] += nurse_wait
        stats["total_bed_wait"] += bed_wait
        stats["total_blood_wait"] += blood_wait
        if env.now > stats["ed_cleared_at"]:
            stats["ed_cleared_at"] = env.now

    def arrivals():
        pid = 1
        while True:
            yield env.timeout(rng.exponential(ARRIVAL_SCALE))
            if env.now > SIM_DURATION:
                break
            treatment = rng.exponential(MEAN_TREATMENT)
            triage = 0 if rng.bernoulli(CRITICAL_PROB) else 1
            env.process(patient(pid, triage, treatment))
            pid += 1

    env.process(restock())
    env.process(arrivals())
    env.run()

    total = stats["critical_treated"] + stats["standard_treated"]
    mean = (lambda s: s / total if total > 0 else 0.0)
    return {
        "critical_treated": stats["critical_treated"],
        "standard_treated": stats["standard_treated"],
        "early_discharged": stats["early_discharged"],
        "blood_bank_waits": stats["blood_bank_waits"],
        "mean_nurse_wait": mean(stats["total_nurse_wait"]),
        "mean_bed_wait": mean(stats["total_bed_wait"]),
        "mean_blood_wait": mean(stats["total_blood_wait"]),
        "ed_cleared_at": stats["ed_cleared_at"],
    }


def events_per_seed(rec, args):
    return 2 * (rec["critical_treated"] + rec["standard_treated"])


if __name__ == "__main__":
    run("hospital", run_seed, events_per_seed)
