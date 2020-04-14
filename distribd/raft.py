import asyncio
import enum
import logging
import math
import random

import aiohttp
from aiohttp import web
from aiohttp.abc import AbstractAccessLogger

from . import config
from .exceptions import LeaderUnavailable
from .jobs import WorkerPool
from .utils.web import run_server

logger = logging.getLogger(__name__)

ELECTION_TIMEOUT_HIGH = 600
ELECTION_TIMEOUT_LOW = ELECTION_TIMEOUT_HIGH / 2

HEARTBEAT_TIMEOUT = 100

# SCALE_FACTOR = 1000
SCALE_FACTOR = 500


class NotALeader(Exception):
    pass


class NodeState(enum.IntEnum):

    FOLLOWER = 1
    CANDIDATE = 2
    LEADER = 3


class RaftAccessLog(AbstractAccessLogger):
    def log(self, request, response, time):
        pass


class Node:
    def __init__(self, identifier, log):
        self.state = NodeState.FOLLOWER
        self.identifier = identifier

        # persistent state
        self.log = log
        self.voted_for = None

        # volatile state
        self.commit_index = 0

        # leader state
        self.remotes = []

        self._pool = WorkerPool()
        self._heartbeat = None

    async def close(self):
        await asyncio.gather(*[peer.close() for peer in self.remotes])
        await self._pool.close()

    async def add_entry(self, entry):
        if self.state != NodeState.LEADER:
            raise NotALeader("Only leader can append to log")
        await self.log.commit(self.log.current_term, entry)
        return self.log.last_term, self.log.last_index

    async def send_action(self, entries):
        if self.state == NodeState.LEADER:
            for entry in entries:
                wait_term, wait_index = await self.add_entry(entry)
        else:
            wait_term, wait_index = await self.leader.send_add_entries(entries)
        return await self.log.wait_for_commit(wait_term, wait_index)

    @property
    def cluster_size(self):
        return len(self.remotes) + 1

    @property
    def quorum(self):
        return math.floor(self.cluster_size / 2) + 1

    @property
    def leader(self):
        for peer in self.remotes:
            if peer.is_leader:
                return peer

        # FIXME Can we wait for there to be a leader???
        raise LeaderUnavailable()

    def add_member(self, identifier):
        node = RemoteNode(identifier)
        self.remotes.append(node)

    def cancel_election_timeout(self):
        if self._heartbeat:
            self._heartbeat.cancel()
            self._heartbeat = None

    def reset_election_timeout(self):
        self.cancel_election_timeout()
        timeout = (
            random.randrange(ELECTION_TIMEOUT_LOW, ELECTION_TIMEOUT_HIGH) / SCALE_FACTOR
        )
        loop = asyncio.get_event_loop()
        self._heartbeat = loop.call_later(timeout, self.become_candidate)

    def become_follower(self):
        self.voted_for = None
        self.reset_election_timeout()

        if self.state == NodeState.FOLLOWER:
            return

        self.state = NodeState.FOLLOWER
        logger.debug("Became follower")

    def become_candidate(self):
        if self.state == NodeState.LEADER:
            logger.debug("Can't become candidate when already leader")
            return

        if self.state == NodeState.CANDIDATE:
            return

        logger.debug("Became candidate")
        self.state = NodeState.CANDIDATE
        self.voted_for = None

        self._pool.spawn(self.do_gather_votes())

    def become_leader(self):
        self.cancel_election_timeout()

        if self.state == NodeState.LEADER:
            return

        self.state = NodeState.LEADER

        logger.debug("Became leader")

        for peer in self.remotes:
            peer.next_index = self.log.last_index + 1
            peer.match_index = 0

        self._pool.spawn(self.do_heartbeats())

        # It's considered "unsafe" to commit a transaction from a previous term
        # But safe to indirectly commit it when commiting a no-op in the current term
        self._pool.spawn(self.add_entry({}))

    async def advance_commit_index(self):
        commit_index = 0
        i = max(self.commit_index, 1)
        while i <= self.log.last_index:
            if self.log[i][0] != self.log.current_term:
                i += 1
                continue

            # Start counting at 1 because we count as a vote
            match_count = 1
            for peer in self.remotes:
                if peer.match_index >= i:
                    match_count += 1

            if match_count >= self.quorum:
                commit_index = i

            i += 1

        if commit_index <= self.commit_index:
            return

        logger.debug(
            "Achieved a quorum - advancing commit index from %d to %d",
            self.commit_index,
            commit_index,
        )
        self.commit_index = commit_index

        await self.log.apply(self.commit_index)

    async def do_gather_votes(self):
        """Try and gather votes from all peers for current term."""
        if self.state != NodeState.CANDIDATE:
            return

        await self.log.set_term(self.log.current_term + 1)
        self.voted_for = self.identifier

        payload = {
            "term": self.log.current_term,
            "candidate_id": self.identifier,
            "last_index": self.log.last_index,
            "last_term": self.log.last_term,
        }

        requests = [node.send_request_vote(payload) for node in self.remotes]

        random_timeout = (
            random.randrange(ELECTION_TIMEOUT_LOW, ELECTION_TIMEOUT_HIGH) / SCALE_FACTOR
        )

        votes = 1
        for response in asyncio.as_completed(requests, timeout=random_timeout):
            try:
                response = await response
            except asyncio.TimeoutError:
                continue
            except Exception:
                logger.exception("Unhandled exception whilst trying to gather votes")
                continue

            # If they are in a newer term than us stop trying to become a leader immediately
            if response["term"] > self.log.current_term:
                self.become_follower()
                await self.log.set_term(response["term"])
                return

            if response["vote_granted"] is True:
                votes += 1

        if self.state != NodeState.CANDIDATE:
            return

        if votes >= self.quorum:
            self.become_leader()
            return

        self.become_follower()

    async def do_heartbeat(self, node):
        if self.state != NodeState.LEADER:
            return

        # The biggest prev_index can be is log.last_index, so cap its size to that.
        # Though the question is, how does it end up bigger than last_index in the first place.
        prev_index = min(node.next_index - 1, self.log.last_index)
        prev_term = self.log[prev_index][0] if prev_index >= 1 else 0
        entries = self.log[node.next_index :]

        payload = {
            "term": self.log.current_term,
            "leader_id": self.identifier,
            "prev_index": prev_index,
            "prev_term": prev_term,
            "entries": entries,
            "leader_commit": self.commit_index,
        }

        result = await node.send_append_entries(payload)

        if result["term"] > self.log.current_term:
            self.become_follower()
            await self.log.set_term(result["term"])
            return

        if not result["success"]:
            if node.next_index > 1:
                node.next_index -= 1
            return

        node.next_index += len(entries)
        node.match_index = node.next_index - 1

        await self.advance_commit_index()

    async def do_heartbeats(self):
        while self.state == NodeState.LEADER:
            for node in self.remotes:
                self._pool.spawn(self.do_heartbeat(node))

            await asyncio.sleep(HEARTBEAT_TIMEOUT / SCALE_FACTOR)

    def find_first_inconsistency(self, ours, theirs):
        for i, (our_entry, their_entry) in enumerate(zip(ours, theirs)):
            if our_entry[0] != their_entry[0]:
                return i
        return min(len(ours), len(theirs))

    async def recv_append_entries(self, request):
        term = request["term"]

        if term > self.log.current_term:
            self.become_follower()
            await self.log.set_term(term)

        if term < self.log.current_term:
            logger.debug(
                "Message received for old term %d, current term is %d",
                term,
                self.log.current_term,
            )
            return False

        # become_follower does this as well. would be nice not to dupe it.
        self.reset_election_timeout()

        prev_index = request["prev_index"]
        prev_term = request["prev_term"]

        if prev_index > self.log.last_index:
            logger.debug("Leader assumed we had log entry %d but we do not", prev_index)
            return False

        if prev_index and self.log[prev_index][0] != prev_term:
            logger.warning(
                "Log not valid - mismatched terms %d and %d at index %d",
                prev_term,
                self.log[prev_index][0],
                prev_index,
            )
            return False

        for peer in self.remotes:
            peer.is_leader = peer.identifier == request["leader_id"]

        # If leader sends us a batch of entries we already have we can avoid truncating
        # if they are actually consistent
        inconsistency_offset = self.find_first_inconsistency(
            self.log[prev_index + 1 :], request["entries"]
        )
        prev_index += inconsistency_offset
        entries = request["entries"][inconsistency_offset:]

        if self.log.last_index > prev_index:
            logger.error("Need to truncate log to recover quorum")
            if not await self.log.rollback(prev_index):
                return False

        for entry in entries:
            await self.log.commit(entry[0], entry[1])

        if request["leader_commit"] > self.commit_index:
            commit_index = min(request["leader_commit"], self.log.last_index)
            self.commit_index = commit_index
            await self.log.apply(self.commit_index)

        return True

    async def recv_request_vote(self, request):
        term = request["term"]
        logger.debug("Received a vote request for term %d", term)

        if term > self.log.current_term:
            self.become_follower()
            await self.log.set_term(term)

        if term < self.log.current_term:
            logger.debug("Vote request rejected as term already over")
            return False

        if self.voted_for and request["candidate_id"] != self.voted_for:
            logger.debug(
                "Vote request rejected as already voted for %s", self.voted_for
            )
            return False

        last_term = request["last_term"]
        if last_term < self.log.last_term:
            logger.debug("Vote request rejected as last term older than current term")
            return False

        last_index = request["last_index"]
        if last_index < self.log.last_index:
            logger.debug("Vote request rejected as last index older than own log")
            return False

        self.voted_for = request["candidate_id"]

        # This is over cautious.
        self.reset_election_timeout()

        return True

    async def run_forever(self, port):
        self.become_follower()
        return await run_server(
            "127.0.0.1", port, routes, access_log_class=RaftAccessLog, node=self
        )


class RemoteNode:
    def __init__(self, identifier):
        self.identifier = identifier
        self.url = config.config[identifier]["raft_url"]
        self.is_leader = False
        self.next_index = 0
        self.match_index = 0
        self.session = aiohttp.ClientSession()

    async def send_add_entries(self, payload):
        try:
            resp = await self.session.post(f"{self.url}/add-entries", json=payload)
        except aiohttp.ClientConnectionError:
            raise NotALeader()

        if resp.status != 200:
            raise NotALeader("Unable to write to this node")
        payload = await resp.json()
        return payload["last_term"], payload["last_index"]

    async def send_append_entries(self, payload):
        try:
            resp = await self.session.post(f"{self.url}/append-entries", json=payload)
        except aiohttp.ClientConnectionError:
            return {"term": 0, "success": False}

        if resp.status != 200:
            return {"term": 0, "success": False}
        return await resp.json()

    async def send_request_vote(self, payload):
        try:
            resp = await self.session.post(f"{self.url}/request-vote", json=payload)
        except aiohttp.ClientConnectionError:
            return {"term": 0, "vote_granted": False}

        if resp.status != 200:
            return {"term": 0, "vote_granted": False}
        return await resp.json()

    async def close(self):
        await self.session.close()


routes = web.RouteTableDef()


@routes.post("/append-entries")
async def append_entries(request):
    node = request.app["node"]

    payload = await request.json()

    return web.json_response(
        {
            "term": node.log.current_term,
            "success": await node.recv_append_entries(payload),
        }
    )


@routes.post("/request-vote")
async def request_vote(request):
    node = request.app["node"]

    payload = await request.json()

    return web.json_response(
        {
            "term": node.log.current_term,
            "vote_granted": await node.recv_request_vote(payload),
        }
    )


@routes.post("/add-entries")
async def add_entry(request):
    node = request.app["node"]

    payload = await request.json()

    if node.state != NodeState.LEADER:
        return web.json_response(
            status=400, reason="Not a leader", json={"reason": "NOT_A_LEADER"},
        )

    for entry in payload:
        last_term, last_index = await node.add_entry(entry)

    return web.json_response({"last_term": last_term, "last_index": last_index})


@routes.get("/status")
async def status(request):
    node = request.app["node"]

    consensus = node.state == NodeState.LEADER or (
        any(peer.is_leader for peer in node.remotes)
    )

    payload = {
        "status": node.state,
        "log_last_index": node.log.last_index,
        "log_last_term": node.log.last_term,
        "applied_index": node.log.applied_index,
        "committed_index": node.commit_index,
        "consensus": consensus,
    }

    return web.json_response(payload)
