import unittest
from ai_providers import get_ai_provider, OllamaProvider, LlamaCppProvider

class TestProviders(unittest.TestCase):
    def test_ollama_local_detection(self):
        local_p = OllamaProvider("http://localhost:11434")
        self.assertTrue(local_p.is_local())

        ip_local = OllamaProvider("http://127.0.0.1:11434")
        self.assertTrue(ip_local.is_local())

    def test_ollama_remote_detection(self):
        remote_p = OllamaProvider("http://192.168.1.100:11434")
        self.assertFalse(remote_p.is_local())

        domain_p = OllamaProvider("https://ollama.mycompany.net")
        self.assertFalse(domain_p.is_local())

    def test_ollama_auth_headers(self):
        p = OllamaProvider("http://remote.host:11434", api_key="secret-token-123")
        headers = p._get_headers()
        self.assertEqual(headers["Authorization"], "Bearer secret-token-123")

    def test_llamacpp_provider(self):
        p = LlamaCppProvider("http://localhost:8080")
        self.assertEqual(p.name, "Llama.cpp")
        self.assertTrue(p.is_local())

        remote_llama = LlamaCppProvider("http://192.168.1.200:8080")
        self.assertFalse(remote_llama.is_local())

    def test_factory_switching(self):
        ollama_cfg = {"backend_type": "ollama", "ollama_url": "http://remote:11434"}
        p1 = get_ai_provider(ollama_cfg)
        self.assertIsInstance(p1, OllamaProvider)
        self.assertEqual(p1.base_url, "http://remote:11434")

        llama_cfg = {"backend_type": "llamacpp", "llamacpp_url": "http://llama-box:8080"}
        p2 = get_ai_provider(llama_cfg)
        self.assertIsInstance(p2, LlamaCppProvider)
        self.assertEqual(p2.base_url, "http://llama-box:8080")

if __name__ == "__main__":
    unittest.main()
