import importlib.util
import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "struggletime_verify_under_test", ROOT / "src" / "verify.py"
)
verify = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify)


class SharedAllowlistCasesTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with (ROOT / "schemas" / "allowlist_cases.json").open(encoding="utf-8") as handle:
            cls.cases = json.load(handle)

    def test_valid_buckets(self):
        for case in self.cases["valid"]:
            with self.subTest(case=case["name"]):
                self.assertEqual(verify.validate_allowlist(case["bucket"]), [])

    def test_invalid_buckets(self):
        for case in self.cases["invalid"]:
            with self.subTest(case=case["name"]):
                self.assertNotEqual(verify.validate_allowlist(case["bucket"]), [])


if __name__ == "__main__":
    unittest.main()
