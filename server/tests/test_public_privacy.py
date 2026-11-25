import importlib.util
import json
import sys
import tempfile
import threading
import time
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "struggletime_server_under_test", ROOT / "server" / "server.py"
)
server = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = server
SPEC.loader.exec_module(server)


class PublicPrivacyTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        self.previous_db = server.DB
        server.DB = str(Path(self.tempdir.name) / "public-privacy.db")
        server._CONN = None
        self.conn = server.db()
        now = int(time.time())
        self.public_token = "public-test-token"
        self.private_token = "private-test-token"
        for handle, token, public, email in (
            ("public-user", self.public_token, 1, "public-private@example.test"),
            ("private-user", self.private_token, 0, "private-secret@example.test"),
        ):
            self.conn.execute(
                """INSERT INTO user(
                       handle, token_hash, is_public, created_at, display_name,
                       email, country, timezone, admin_level, trust_level
                   ) VALUES (?,?,?,?,?,?,?,?,?,?)""",
                (
                    handle,
                    server.hash_token(token),
                    public,
                    now,
                    f"Display {handle}",
                    email,
                    "Private Country",
                    "Private/Timezone",
                    "admin",
                    "blue",
                ),
            )
        self.conn.execute(
            """INSERT INTO link(
                   handle, github_login, slack_user_id, slack_avatar, slack_name
               ) VALUES (?,?,?,?,?)""",
            (
                "public-user",
                "public-github",
                "private-slack-user-id",
                "https://example.test/avatar.png",
                "Public Slack Name",
            ),
        )
        self.private_path = r"C:\Users\Example\secret-project"
        self.private_email = "public-private@example.test"
        self.removed_repo = "https://github.com/example/removed-secret.git"
        self.conn.execute(
            """INSERT INTO project(
                   id, owner, name, repo_url, head_from, head_to,
                   local_path, last_repo_url
               ) VALUES (?,?,?,?,?,?,?,?)""",
            (
                "public-project",
                "public-user",
                "Public Project",
                "https://github.com/example/public-project",
                "abc123",
                "def456",
                self.private_path,
                self.removed_repo,
            ),
        )
        self.conn.commit()

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), server.H)
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()
        self.base = f"http://127.0.0.1:{self.httpd.server_address[1]}"

    def tearDown(self):
        self.httpd.shutdown()
        self.httpd.server_close()
        self.thread.join(timeout=5)
        self.conn.close()
        server._CONN = None
        server.DB = self.previous_db
        self.tempdir.cleanup()

    def get(self, path, token=None):
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        request = Request(f"{self.base}{path}", headers=headers)
        try:
            with urlopen(request, timeout=5) as response:
                return response.status, json.load(response)
        except HTTPError as error:
            try:
                return error.code, json.load(error)
            finally:
                error.close()

    def test_public_profile_excludes_private_fields(self):
        status, payload = self.get("/v1/users/public-user")

        self.assertEqual(status, 200)
        self.assertEqual(payload["handle"], "public-user")
        self.assertEqual(payload["display_name"], "Display public-user")
        self.assertEqual(payload["github_login"], "public-github")
        self.assertEqual(payload["projects"][0]["repo_slug"], "example/public-project")
        self.assertEqual(
            set(payload["projects"][0]),
            {
                "id",
                "owner",
                "name",
                "repo_url",
                "repo_slug",
                "head_from",
                "head_to",
                "archived",
                "deleted",
            },
        )
        for key in (
            "email",
            "country",
            "timezone",
            "slack_user_id",
            "admin_level",
            "trust_level",
            "api_key",
            "api_key_prefix",
            "has_api_key",
            "install_keys",
            "local_path",
            "last_repo_url",
        ):
            self.assertNotIn(key, payload)
        serialized = json.dumps(payload)
        for private_value in (
            self.private_email,
            self.private_path,
            self.removed_repo,
            "private-slack-user-id",
            "Private Country",
            "Private/Timezone",
        ):
            self.assertNotIn(private_value, serialized)

    def test_owner_endpoints_keep_private_fields(self):
        status, profile = self.get("/v1/auth/me", self.public_token)
        self.assertEqual(status, 200)
        self.assertEqual(profile["email"], self.private_email)
        self.assertEqual(profile["country"], "Private Country")
        self.assertEqual(profile["timezone"], "Private/Timezone")

        status, projects = self.get("/v1/projects", self.public_token)
        self.assertEqual(status, 200)
        project = next(row for row in projects["projects"] if row["id"] == "public-project")
        self.assertEqual(project["local_path"], self.private_path)
        self.assertEqual(project["last_repo_url"], self.removed_repo)

    def test_private_profile_access_rules_remain(self):
        status, _ = self.get("/v1/users/private-user")
        self.assertEqual(status, 403)

        status, _ = self.get("/v1/users/private-user", self.public_token)
        self.assertEqual(status, 403)

        status, payload = self.get("/v1/users/private-user", self.private_token)
        self.assertEqual(status, 200)
        self.assertEqual(payload["handle"], "private-user")
        self.assertNotIn("email", payload)


if __name__ == "__main__":
    unittest.main()
