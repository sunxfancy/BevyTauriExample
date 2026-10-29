import unittest
from PIL import Image, ImageDraw
from desktop_smoke import Smoke, macos_accessibility_trusted
from unittest.mock import Mock, patch


class RenderAssertionsTest(unittest.TestCase):
    def setUp(self):
        self.smoke = object.__new__(Smoke)
        self.state = dict(width=320, height=240,
                          logo=dict(x=10, y=10, width=100, height=80),
                          panel=dict(x=0, y=0, width=320, height=240))

    def frame(self, ui=True, scene=True):
        image = Image.new('RGB', (320, 240), 'black')
        draw = ImageDraw.Draw(image)
        if ui:
            draw.rectangle((20, 20, 70, 60), fill=(30, 200, 220))
        if scene:
            draw.rectangle((150, 100, 190, 140), fill=(60, 180, 70))
        return image

    def test_missing_either_layer_fails(self):
        for ui, scene in [(False, False), (False, True), (True, False)]:
            with self.subTest(ui=ui, scene=scene):
                self.assertFalse(self.smoke.visible(self.frame(ui, scene), self.state)[0])

    def test_both_layers_pass_at_native_and_retina_scale(self):
        for scale in [1, 2]:
            image = self.frame().resize((320*scale, 240*scale))
            self.assertTrue(self.smoke.visible(image, self.state)[0])

    def test_cyan_outside_the_react_logo_does_not_pass(self):
        image = self.frame(ui=False)
        ImageDraw.Draw(image).rectangle((240, 10, 300, 70), fill=(30, 200, 220))
        self.assertFalse(self.smoke.visible(image, self.state)[0])


class MacAccessibilityTest(unittest.TestCase):
    def test_native_application_services_permission_result_is_preserved(self):
        for allowed in [True, False]:
            library = Mock()
            library.AXIsProcessTrusted.return_value = allowed
            with patch('desktop_smoke.ctypes.CDLL', return_value=library) as load:
                self.assertEqual(macos_accessibility_trusted(), allowed)
                load.assert_called_once_with(
                    '/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices'
                )
                library.AXIsProcessTrusted.assert_called_once_with()


if __name__ == '__main__':
    unittest.main()
