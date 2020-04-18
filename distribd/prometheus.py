from aiohttp import web
from prometheus_client import CONTENT_TYPE_LATEST, generate_latest
from prometheus_client.core import CollectorRegistry, GaugeMetricFamily

from .utils.web import run_server

routes = web.RouteTableDef()


class MetricsCollector:
    def __init__(self, node):
        self.node = node

    def collect(self):
        # last_applied = GaugeMetricFamily(
        #    "distribd_last_applied_index",
        #    "Last index that was applied",
        #    labels=["identifier"],
        # )
        # last_applied.add_metric([self.node.identifier], self.node.log.applied_index)
        # yield last_applied

        last_committed = GaugeMetricFamily(
            "distribd_last_committed_index",
            "Last index that was committed",
            labels=["identifier"],
        )
        last_committed.add_metric([self.node.identifier], self.node.commit_index)
        yield last_committed

        last_saved = GaugeMetricFamily(
            "distribd_last_saved_index",
            "Last index that was stored in the commit log",
            labels=["identifier"],
        )
        last_saved.add_metric([self.node.identifier], self.node.log.last_index)
        yield last_saved

        last_term_saved = GaugeMetricFamily(
            "distribd_last_saved_term",
            "Last term that was stored in the commit log",
            labels=["identifier"],
        )
        last_term_saved.add_metric([self.node.identifier], self.node.log.last_term)
        yield last_term_saved

        current_term = GaugeMetricFamily(
            "distribd_current_term",
            "The current term for a node",
            labels=["identifier"],
        )
        current_term.add_metric([self.node.identifier], self.node.term)
        yield current_term

        current_state = GaugeMetricFamily(
            "distribd_current_state",
            "The current state for a node",
            labels=["identifier", "state"],
        )
        current_state.add_metric([self.node.identifier, str(self.node.state)], 1)
        yield current_state


@routes.get("/metrics")
async def metrics(request):
    return web.Response(
        body=generate_latest(request.app["prometheus_registry"]),
        headers={"Content-Type": CONTENT_TYPE_LATEST},
    )


@routes.get("/healthz")
async def ok(request):
    return web.json_response({"ok": True})


async def run_prometheus(identifier, registry_state, images_directory, node, port):
    registry = CollectorRegistry()
    collector = MetricsCollector(node)
    registry.register(collector)

    return await run_server(
        "0.0.0.0",
        port,
        routes,
        identifier=identifier,
        registry_state=registry_state,
        images_directory=images_directory,
        node=node,
        prometheus_registry=registry,
    )
