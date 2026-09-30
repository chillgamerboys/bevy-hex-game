"""The generated object manifest must satisfy runtime catalog admission."""
import json
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

import forest_finish


def catalog_ids(source):
    body = re.search(r'objects\s*:\s*\[(.*?)\]', source, re.DOTALL).group(1)
    return json.loads('[' + body.rstrip().removesuffix(',') + ']')


class ExpeditionCatalog(unittest.TestCase):
    def test_additive_generation_retains_legacy_assets_and_is_strictly_sorted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            art = root / 'assets/art'
            art.mkdir(parents=True)
            manifest = art / 'object_catalog.ron'
            manifest.write_text('(schema_version:1,objects:["prop/z", "plant/a", "plant/a",])\n')
            (art / 'voxel_styles.ron').write_text('(schema_version:1,styles:{})\n')
            outputs = {'assets/art/objects/plant/new.ron': 'new blueprint',
                       'assets/art/objects/plant/a.ron': 'existing blueprint'}
            with patch.object(forest_finish, 'ROOT', root):
                forest_finish.write_assets(outputs, {})
                self.assertEqual(catalog_ids(manifest.read_text()), ['plant/a', 'plant/new', 'prop/z'])
                first = manifest.read_bytes()
                forest_finish.write_assets(outputs, {})
                self.assertEqual(manifest.read_bytes(), first)
            self.assertEqual((art / 'objects/plant/new.ron').read_text(), 'new blueprint')

    def test_committed_runtime_manifest_already_meets_the_public_order_contract(self):
        source = (forest_finish.ROOT / 'assets/art/object_catalog.ron').read_text()
        ids = catalog_ids(source)
        self.assertTrue(all(a < b for a, b in zip(ids, ids[1:])))
        self.assertIn('plant/forest-heart', ids)
        self.assertIn('plant/forest-expedition-heart', ids)
        self.assertIn('prop/expedition-bridge-portal-east', ids)
        self.assertIn('prop/snowy-grass-tuft', ids)


if __name__ == '__main__':
    unittest.main()
