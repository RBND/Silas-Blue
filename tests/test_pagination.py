import unittest
from bot_core import paginate_text

class TestPagination(unittest.TestCase):
    def test_short_text(self):
        text = "Hello world!"
        pages = paginate_text(text, 1900)
        self.assertEqual(len(pages), 1)
        self.assertEqual(pages[0], "Hello world!")

    def test_long_text_split(self):
        # 5000 characters of text
        text = "This is a word " * 400
        pages = paginate_text(text, 1850)
        self.assertGreater(len(pages), 1)
        for page in pages:
            self.assertLessEqual(len(page), 1850)
            # Verify combined length with Discord header is strictly < 2000
            header = f"Page 99/99\n{page}"
            self.assertLessEqual(len(header), 2000)

    def test_unbroken_long_line(self):
        # Single continuous line of 4000 characters without spaces
        text = "A" * 4000
        pages = paginate_text(text, 1850)
        self.assertGreater(len(pages), 1)
        for page in pages:
            self.assertLessEqual(len(page), 1850)

if __name__ == "__main__":
    unittest.main()
