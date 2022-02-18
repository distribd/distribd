from confuse import Configuration

from distribd.seeding import Seeder


def test_exchange_receive_initial():
    c = Configuration("distribd", __name__)
    c["node"]["identifier"].set("node1")
    c["raft"]["address"].set("127.0.0.1")
    c["raft"]["port"].set(8080)
    c["registry"]["default"]["address"].set("127.0.0.1")
    c["registry"]["default"]["port"].set(9080)

    s = Seeder(c, 1, None)
    s.process_state_change(
        {
            "raft": {"address": "127.0.0.1", "port": 8080},
            "registry.default": {"address": "127.0.0.1", "port": 9080},
        }
    )

    result = s.exchange_gossip(
        {
            "node2": {
                "raft": {"address": "127.0.0.1", "port": 8081},
                "registry": {"address": "127.0.0.1", "port": 9081},
                "generation": 0,
            }
        }
    )

    assert result == {
        "node1": {
            "raft": {"address": "127.0.0.1", "port": 8080},
            "registry": {"address": "127.0.0.1", "port": 9080},
            "generation": 1,
        }
    }


def test_exchange_receive_ancestor():
    c = Configuration("distribd", __name__)
    c["node"]["identifier"].set("node1")
    c["raft"]["address"].set("127.0.0.1")
    c["raft"]["port"].set(8080)
    c["registry"]["default"]["address"].set("127.0.0.1")
    c["registry"]["default"]["port"].set(9080)

    s = Seeder(c, 1, None)
    s.process_state_change(
        {
            "raft": {"address": "127.0.0.1", "port": 8080},
            "registry.default": {"address": "127.0.0.1", "port": 9080},
        }
    )

    result = s.exchange_gossip(
        {
            "node1": {
                "raft": {"address": "127.0.0.1", "port": 8081},
                "registry": {"address": "127.0.0.1", "port": 9081},
                "generation": 25,
            }
        }
    )

    assert result == {
        "node1": {
            "raft": {"address": "127.0.0.1", "port": 8080},
            "registry": {"address": "127.0.0.1", "port": 9080},
            "generation": 1,
        }
    }
