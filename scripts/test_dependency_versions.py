"""DX12 passes COM objects between wgpu-hal and gpu-allocator.
Their windows-rs versions must match even though allocator allows a wide range.
"""
from pathlib import Path
import tomllib
import unittest


class WindowsDependencyTest(unittest.TestCase):
    def test_dx12_allocator_and_renderer_use_the_same_windows_types(self):
        lock = Path(__file__).resolve().parents[1] / 'src-tauri' / 'Cargo.lock'
        packages = tomllib.loads(lock.read_text(encoding='utf-8'))['package']
        by_id = {f"{p['name']} {p['version']}": p for p in packages}
        checked = 0
        for renderer in packages:
            if renderer['name'] != 'wgpu-hal':
                continue
            allocator_id = next((d for d in renderer.get('dependencies', [])
                                 if d.startswith('gpu-allocator ')), None)
            if not allocator_id:
                continue
            allocator = by_id[allocator_id]
            windows = lambda package: next(d for d in package['dependencies']
                                           if d.startswith('windows '))
            with self.subTest(wgpu_hal=renderer['version']):
                self.assertEqual(windows(renderer), windows(allocator))
            checked += 1
        self.assertGreater(checked, 0, 'No DX12 dependency edges were checked')


if __name__ == '__main__':
    unittest.main()
