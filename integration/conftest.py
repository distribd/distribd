import time

import httpx
import pytest
from pytest_docker_tools import build, container, fetch, network, volume

temporary_network = network(scope="session")

DISTRIBD_CONFIG = b"""
raft:
    address: 0.0.0.0
    port: 8080

registry:
    default:
        address: 0.0.0.0
        port: 9080

        token_server:
            enabled: false

            realm: http://docker_auth:5001/auth
            service: My registry
            issuer: My issuer
            public_key: token_server.pub


prometheus:
    address: 0.0.0.0
    port: 7080

storage: var

peers:
    - name: node1
      address: node1
      port: 8080

    - name: node2
      address: node2
      port: 8080

    - name: node3
      address: node3
      port: 8080

"""

DOCKER_AUTH_CONFIG = b"""
server:
  addr: ":5001"

token:
  issuer: "Acme auth server"  # Must match issuer in the Registry config.
  expiration: 900
  certificate: /config/server.cert
  key: /config/server.key

users:
  "admin":
    password: "$2y$05$LO.vzwpWC5LZGqThvEfznu8qhb5SGqvBSWY1J3yZ4AxtMRZ3kN5jC"
  "test":
    password: "$2y$05$WuwBasGDAgr.QCbGIjKJaep4dhxeai9gNZdmBnQXqpKly57oNutya"

acl:
- match:
    account: "admin"
  actions: ["*"]
  comment: "Admin has full access to everything."
- match:
    account: "test"
  actions: ["pull"]
  comment: "User test can pull stuff."

"""

DOCKER_AUTH_PUBLIC = b"""
-----BEGIN CERTIFICATE-----
MIIBEjCBuAIJAOStacpfM+zAMAoGCCqGSM49BAMCMBExDzANBgNVBAMMBnVudXNl
ZDAeFw0yMDA0MjAxNzAxNTVaFw0yMDA1MjAxNzAxNTVaMBExDzANBgNVBAMMBnVu
dXNlZDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABORsOZ3ZGXyxduh6uq8CNAnJ
SUY2H3ijQh1EYhKNU7R6egf3xdJWh92ekVOHlDJZ3xY954gi+C1a7IocHdtonzgw
CgYIKoZIzj0EAwIDSQAwRgIhAMwrxOl/s3IJGHSEDd5VMbIFaaPT1mO/1ymHnu/O
j+6rAiEAoZXWaKucFpvqkkbrURjjyYZJGfClWkB9vZsJVDxKsUI=
-----END CERTIFICATE-----
""".strip()


DOCKER_AUTH_PRIVATE = b"""
-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIFg2FbPjqQ/yu5XuMH53ol0cjsKEvX0Zn2yYPWcJxcrpoAoGCCqGSM49
AwEHoUQDQgAE5Gw5ndkZfLF26Hq6rwI0CclJRjYfeKNCHURiEo1TtHp6B/fF0laH
3Z6RU4eUMlnfFj3niCL4LVrsihwd22ifOA==
-----END EC PRIVATE KEY-----
""".strip()

docker_auth_image = fetch(repository="cesanta/docker_auth:1.9.0")

docker_auth_config = volume(
    initial_content={
        "auth_config.yml": DOCKER_AUTH_CONFIG,
        "server.cert": DOCKER_AUTH_PUBLIC,
        "server.key": DOCKER_AUTH_PRIVATE,
    },
    scope="session",
)
docker_auth_logs = volume(
    scope="session",
)

docker_auth = container(
    hostname="docker_auth",
    image="{docker_auth_image.id}",
    scope="session",
    volumes={
        "{docker_auth_config.name}": {"bind": "/config"},
        "{docker_auth_logs.name}": {"bind": "/logs"},
    },
    ports={
        "5001/tcp": None,
    },
    network="{temporary_network.name}",
)

distribd_image = build(path=".")

distribd_config = volume(
    initial_content={
        "config.yaml": DISTRIBD_CONFIG,
    },
    scope="session",
)

node1 = container(
    hostname="node1",
    image="{distribd_image.id}",
    scope="session",
    command=["/app/bin/python", "-m", "distribd", "--name", "node1"],
    environment={
        "ROCKET_ADDRESS": "0.0.0.0",
        "ROCKET_LOG_LEVEL": "debug",
    },
    volumes={
        "{distribd_config.name}": {"bind": "/root/.config/distribd"},
    },
    ports={
        "8080/tcp": None,
    },
    network="{temporary_network.name}",
)

node2 = container(
    hostname="node2",
    image="{distribd_image.id}",
    scope="session",
    command=["/app/bin/python", "-m", "distribd", "--name", "node2"],
    environment={
        "ROCKET_ADDRESS": "0.0.0.0",
    },
    volumes={
        "{distribd_config.name}": {"bind": "/root/.config/distribd"},
    },
    ports={
        "8080/tcp": None,
    },
    network="{temporary_network.name}",
)

node3 = container(
    hostname="node3",
    image="{distribd_image.id}",
    scope="session",
    command=["/app/bin/python", "-m", "distribd", "--name", "node3"],
    environment={
        "ROCKET_ADDRESS": "0.0.0.0",
    },
    volumes={
        "{distribd_config.name}": {"bind": "/root/.config/distribd"},
    },
    ports={
        "8080/tcp": None,
    },
    network="{temporary_network.name}",
)


@pytest.fixture()
def cluster(docker_auth, node1, node2, node3):
    while True:
        for node in [node1, node2, node3]:
            ip, port = node.get_addr("8080/tcp")
            resp = httpx.get(f"http://{ip}:{port}/status").json()
            print(resp)
            if not resp["stable"] or not resp["consensus"]:
                break
        else:
            return

        time.sleep(1)
