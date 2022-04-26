"""
Unit tests for the state machine

These are testing the state machine in **isolation**. That doesn't mean it works as a whole, so
make sure to build good integration tests as well.
"""

from distribd.machine import Machine, Message, Msg, NodeState


def test_become_pre_candidate(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")

    m.tick = 0
    m.step(Msg("node1", "node1", Message.Tick, 0))

    assert m.state == NodeState.PRE_CANDIDATE
    assert len(m.outbox) == 2

    assert m.outbox[0].type == Message.PreVote
    assert m.outbox[1].type == Message.PreVote


def test_pre_candidate_timeout(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")

    m.tick = 0
    m.step(Msg("node1", "node1", Message.Tick, 0))

    assert m.state == NodeState.PRE_CANDIDATE

    m.outbox = []
    m.tick = 0

    m.step(Msg("node1", "node1", Message.Tick, 0))

    assert m.state == NodeState.FOLLOWER


def test_become_candidate(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")

    m.tick = 0
    m.step(Msg("node1", "node1", Message.Tick, 0))

    # One vote is enough to win a prevote and make us a candidate
    m.step(m.outbox.pop(0).reply(1, reject=False))

    assert m.state == NodeState.CANDIDATE
    assert len(m.outbox) == 2

    assert m.outbox[0].type == Message.Vote
    assert m.outbox[1].type == Message.Vote


def test_candidate_timeout(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")

    m.step(Msg("node1", "node1", Message.Tick, 0))

    # One vote is enough to win a prevote and make us a candidate
    m.step(m.outbox.pop(0).reply(1, reject=False))
    assert m.state == NodeState.CANDIDATE

    m.step(Msg("node1", "node1", Message.Tick, 0))

    assert m.state == NodeState.FOLLOWER


def test_become_leader(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")

    m.tick = 0
    m.step(Msg("node1", "node1", Message.Tick, 0))
    m.step(Msg("node2", "node1", Message.PreVoteReply, 1, reject=False))
    m.step(Msg("node3", "node1", Message.PreVoteReply, 1, reject=False))

    assert m.vote_count == 1
    assert m.state == NodeState.CANDIDATE

    # If a node rejects then we should still be a candidate
    m.step(Msg("node3", "node1", Message.VoteReply, 1, reject=True))
    assert m.vote_count == 1
    assert m.state == NodeState.CANDIDATE

    # 3 node cluster, so one node replying is enough to get a quorum
    m.step(Msg("node2", "node1", Message.VoteReply, 1, reject=False))

    assert m.state == NodeState.LEADER
    assert len(m.outbox) == 2

    assert m.peers["node2"].next_index == 1
    assert m.peers["node2"].match_index == 0

    assert m.peers["node3"].next_index == 1
    assert m.peers["node3"].match_index == 0

    assert m.outbox[0].type == Message.AppendEntries
    assert m.outbox[1].type == Message.AppendEntries

    # Further votes should have no effect
    m.step(Msg("node3", "node1", Message.VoteReply, 1, reject=True))

    assert m.vote_count == 2
    assert m.state == NodeState.LEADER
    assert len(m.outbox) == 0

    assert m.peers["node2"].next_index == 1
    assert m.peers["node2"].match_index == 0

    assert m.peers["node3"].next_index == 1
    assert m.peers["node3"].match_index == 0


def test_leader_handle_append_entries_reply_success(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")

    m.log.append((1, {}))
    m.log.append((1, {}))
    m.log.append((1, {}))

    assert m.log.last_index == 3
    assert m.log.last_term == 1

    m.tick = 0
    m.step(Msg("node1", "node1", Message.Tick, 0))

    m.step(Msg("node2", "node1", Message.PreVoteReply, 1, reject=False))
    m.step(Msg("node3", "node1", Message.PreVoteReply, 1, reject=False))

    m.step(Msg("node2", "node1", Message.VoteReply, 1, reject=False))
    outbox = list(m.outbox)
    m.step(Msg("node3", "node1", Message.VoteReply, 1, reject=False))
    outbox.extend(m.outbox)

    m.step(outbox[0].reply(1, reject=False, log_index=3))
    m.step(outbox[1].reply(1, reject=False, log_index=3))

    assert m.peers["node2"].next_index == 4
    assert m.peers["node2"].match_index == 3

    # Make sure we can't commit what we don't have
    m.step(outbox[0].reply(1, reject=False, log_index=10))
    m.step(outbox[1].reply(1, reject=False, log_index=10))

    # These have gone up one because the leader has committed an empty log entry
    # As it has started a new term.
    assert m.peers["node2"].next_index == 5
    assert m.peers["node2"].match_index == 4


def test_append_entries_against_empty(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")

    m.tick = 0

    m.step(
        Msg(
            "node2",
            "node1",
            Message.AppendEntries,
            2,
            prev_index=0,
            prev_term=0,
            entries=[(2, {})],
            leader_commit=0,
        )
    )

    # Should reset election timeout
    assert m.tick > 0

    assert m.state == NodeState.FOLLOWER
    assert m.obedient is True
    assert m.leader == "node2"
    assert m.log[1:] == [(2, {})]
    assert m.commit_index == 0

    assert m.outbox[0].type == Message.AppendEntriesReply


def test_answer_pre_vote(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")
    m.term = 1

    # Vote rejected because in same term and obedient
    m.step(Msg("node2", "node1", Message.PreVote, 1, index=1))
    assert m.outbox[-1].type == Message.PreVoteReply
    assert m.outbox[-1].reject is True

    # Becomes a PRE_CANDIDATE - no longer obedient
    m.tick = 0
    m.step(Msg("node1", "node1", Message.Tick, 0))
    assert m.obedient is False
    assert m.state == NodeState.PRE_CANDIDATE
    assert m.term == 1

    # In a later term and not obedient
    m.tick = 0
    m.step(Msg("node2", "node1", Message.PreVote, 2, index=1))
    assert m.outbox[-1].type == Message.PreVoteReply
    assert m.outbox[-1].reject is False

    # Hasn't actually voted so this shouldn't be set
    assert m.voted_for is None


def test_answer_vote(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")
    m.term = 1

    # Vote rejected because in same term
    m.step(Msg("node2", "node1", Message.Vote, 1, index=1))
    assert m.outbox[-1].type == Message.VoteReply
    assert m.outbox[-1].reject is True

    # Vote in new term, but it is still obedient to current leader
    m.step(Msg("node2", "node1", Message.Vote, 2, index=1))
    assert m.outbox[-1].type == Message.VoteReply
    assert m.outbox[-1].reject is True

    # Becomes a PRE_CANDIDATE - nog longer obedient
    m.tick = 0
    m.step(Msg("node1", "node1", Message.Tick, 0))
    assert m.obedient is False
    assert m.state == NodeState.PRE_CANDIDATE
    assert m.term == 1

    # Vote in new term, but it is still obedient to current leader
    m.tick = 0
    m.step(Msg("node2", "node1", Message.Vote, 2, index=1))
    assert m.term == 2
    assert m.outbox[-1].type == Message.VoteReply
    assert m.outbox[-1].reject is False

    # Election timer reset after vote
    assert m.tick > 0

    # Pin to node until next reset
    assert m.voted_for == "node2"

    # Term should have increased
    assert m.term == 2


def test_append_entries_revoke_previous_log_entry(event_loop):
    m = Machine("node1")
    m.add_peer("node2")
    m.add_peer("node3")
    m.term = 2

    # Recovered from saved log
    m.log.append((1, {"type": "consensus"}))

    # Committed when became leader
    m.log.append((2, {}))

    m.step(
        Msg(
            "node2",
            "node1",
            Message.AppendEntries,
            term=3,
            prev_index=2,
            prev_term=3,
            entries=[],
            leader_commit=0,
        )
    )

    assert m.log[2] == (2, {})
    assert m.outbox[-1].type == Message.AppendEntriesReply
    assert m.outbox[-1].reject is True

    m.step(
        Msg(
            "node2",
            "node1",
            Message.AppendEntries,
            term=3,
            prev_index=1,
            prev_term=1,
            entries=[(3, {})],
            leader_commit=0,
        )
    )

    assert m.log[2] == (3, {})
    assert m.outbox[-1].type == Message.AppendEntriesReply
    assert m.outbox[-1].reject is False
    assert m.outbox[-1].log_index == 2


def test_find_inconsistencies(event_loop):
    n = Machine("node1")
    assert (
        n.find_first_inconsistency(
            [(1, None), (1, None), (1, None)], [(2, None), (2, None), (3, None)]
        )
        == 0
    )

    assert (
        n.find_first_inconsistency(
            [(1, None), (1, None), (1, None)], [(1, None), (2, None), (3, None)]
        )
        == 1
    )

    assert (
        n.find_first_inconsistency(
            [(1, None), (1, None), (1, None)], [(1, None), (1, None), (3, None)]
        )
        == 2
    )

    assert (
        n.find_first_inconsistency(
            [(1, None), (1, None), (1, None)], [(1, None), (1, None), (1, None)]
        )
        == 3
    )

    assert (
        n.find_first_inconsistency(
            [(1, None), (1, None), (1, None), (1, None)],
            [(1, None), (1, None), (1, None)],
        )
        == 3
    )

    assert (
        n.find_first_inconsistency(
            [(1, None), (1, None), (1, None)],
            [(1, None), (1, None), (1, None), (1, None)],
        )
        == 3
    )
