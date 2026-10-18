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

    def test_detector_probe_invalid_port(self):
        from ai_providers.detector import probe_tcp_port
        # Port 59999 should not be listening
        self.assertFalse(probe_tcp_port("127.0.0.1", 59999, timeout=0.05))

    def test_detector_installed_llamacpp(self):
        from ai_providers.detector import detect_installed_llamacpp
        # Should return a string or None without crashing
        res = detect_installed_llamacpp()
        self.assertTrue(res is None or isinstance(res, str))

    def test_detector_local_models(self):
        from ai_providers.detector import detect_local_models
        models = detect_local_models()
        self.assertIsInstance(models, list)

    def test_detect_all_ai_providers_structure(self):
        from ai_providers.detector import detect_all_ai_providers
        data = detect_all_ai_providers()
        self.assertIn("llamacpp", data)
        self.assertIn("ollama", data)
        self.assertIn("recommended_backend", data)
        self.assertIn("running", data["llamacpp"])
        self.assertIn("url", data["llamacpp"])
        self.assertIn("running", data["ollama"])

    def test_factory_auto_mode(self):
        p = get_ai_provider({"backend_type": "auto"})
        self.assertIsNotNone(p)
        self.assertIn(p.name, ("Ollama", "Llama.cpp"))

    def test_llamacpp_model_resolution(self):
        p = LlamaCppProvider("http://127.0.0.1:9931")
        models = p.list_models_sync()
        self.assertIsInstance(models, list)
        self.assertTrue(len(models) > 0)

if __name__ == "__main__":
    unittest.main()
