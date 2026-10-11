import unittest
import importlib.util
from mlx_startup import evaluate_weights


class Array:
    def __init__(self, size):
        self.nbytes = size
        self.done = False


class ProgressTests(unittest.TestCase):
    def test_counts_only_completed_evaluation_and_preserves_all_arrays(self):
        arrays = [Array(3), Array(5), Array(2)]
        observed = []
        calls = []
        def evaluate(*batch):
            calls.extend(batch)
            for array in batch:
                array.done = True
        def report(done, total):
            self.assertEqual(done, sum(a.nbytes for a in arrays if a.done))
            self.assertEqual(total, 10)
            observed.append(done)
        evaluate_weights(evaluate, {"layer": arrays, "unused": None}, report, 8)
        self.assertEqual(calls, arrays)
        self.assertEqual(observed, [0, 8, 10])

    def test_failure_never_reports_full_completion(self):
        observed = []
        def evaluate(*batch):
            raise RuntimeError("allocation failed")
        with self.assertRaisesRegex(RuntimeError, "allocation failed"):
            evaluate_weights(evaluate, {"weights": Array(10)}, lambda d, t: observed.append(d))
        self.assertEqual(observed, [0])

    def test_empty_parameters_preserve_original_call(self):
        self.assertEqual(evaluate_weights(lambda x: x, {}, lambda d, t: self.fail()), {})

    def test_shared_parameters_are_counted_once(self):
        array = Array(10)
        calls = []
        totals = []
        evaluate_weights(lambda *batch: calls.extend(batch), {"a": array, "alias": array}, lambda d, t: totals.append(t))
        self.assertEqual(calls, [array])
        self.assertEqual(totals, [10, 10])

    @unittest.skipUnless(importlib.util.find_spec("mlx"), "native MLX checked on macOS release runner")
    def test_native_mlx_values_are_unchanged_after_measured_evaluation(self):
        import mlx.core as mx
        mx.set_default_device(mx.cpu)
        arrays = [mx.arange(32), mx.arange(16) * 3]
        totals = []
        evaluate_weights(mx.eval, {"layers": arrays}, lambda d, t: totals.append((d, t)), 64)
        self.assertEqual(arrays[0].tolist(), list(range(32)))
        self.assertEqual(arrays[1].tolist(), [n * 3 for n in range(16)])
        self.assertEqual(totals[-1], (sum(a.nbytes for a in arrays),) * 2)


if __name__ == "__main__":
    unittest.main()
