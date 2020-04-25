import logging

import confuse
from jwt import decode
from jwt.exceptions import InvalidTokenError

from .. import exceptions

logger = logging.getLogger(__name__)


class TokenChecker:
    def __init__(self, config: confuse.Configuration):
        self._config = config
        self._enabled = self._config["token_server"]["enabled"].get(bool)

        if self._enabled:
            self._realm = self._config["token_server"]["realm"]
            self._service = self._config["token_server"]["service"]

            self._public_key_path = self._config["token_server"]["public_key"].as_path()
            with open(self._public_key_path, "r") as fp:
                self._public_key = fp.read()

    def authenticate(self, request, repository=None, actions=None):
        if not self._enabled:
            return True

        if "Authorization" not in request.headers:
            raise exceptions.Unauthorized(
                self._realm, self._service, repository, actions
            )

        auth_header = request.headers["Authorization"]
        if not auth_header.startswith("Bearer "):
            raise exceptions.Unauthorized(
                self._realm, self._service, repository, actions
            )

        bearer_token = auth_header.split(" ", 1)[1]

        try:
            decoded = decode(
                bearer_token,
                self._public_key,
                algorithms="ES256",
                audience=self._service,
            )
        except InvalidTokenError as e:
            logger.warning("Request denied due to invalid token: %s", str(e))
            raise exceptions.Denied()

        for access in decoded["access"]:
            if access.get("type") != "repository":
                continue
            if access.get("name") != repository:
                continue
            if not set(actions).issubset(set(access.get("actions", []))):
                continue

            return True

        raise exceptions.Denied()