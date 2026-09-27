"""Geography identity and plain-stage source closure; no Cargo required."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parent))
import grand_package


class GeographicAuthoringTests(unittest.TestCase):
    def fixture(self, root):
        for name in ('crates/hex_world_tool/src/grand.rs', 'assets/config/v4/grand-v4/forest/trees.ron'):
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture')
        source = root / 'world.ron'
        source.write_text('(full_dressing: true, geography: Some("geography-r02.json"))')
        (root / 'geography-r02.json').write_text('{"revision":2}')
        return source

    def test_geography_changes_rotate_identity_and_plain_stage_retains_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = self.fixture(root)
            with patch.object(grand_package, 'ROOT', root):
                before = grand_package.signature(source)
                geography = root / 'geography-r02.json'
                geography.write_text('{"revision":3}')
                self.assertNotEqual(before, grand_package.signature(source))
                stage = root / 'stage'
                stage.mkdir()
                plain = grand_package.stage_plain_source(source, stage)
                self.assertIn('full_dressing:false', plain.read_text())
                self.assertIn('full_dressing: true', source.read_text())
                self.assertEqual((stage / geography.name).read_bytes(), geography.read_bytes())
                self.assertEqual([p.name for p in grand_package.authoring_files(plain)],
                                 ['world.ron', 'geography-r02.json'])

    def test_named_missing_or_nonlocal_dependency_fails_before_compilation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = self.fixture(root)
            (root / 'geography-r02.json').unlink()
            with self.assertRaisesRegex(RuntimeError, 'Missing Grand geography'):
                grand_package.authoring_files(source)
            source.write_text('(full_dressing:true, geography: Some("../geo.json"))')
            with self.assertRaisesRegex(RuntimeError, 'sibling JSON'):
                grand_package.authoring_files(source)


if __name__ == '__main__':
    unittest.main()
