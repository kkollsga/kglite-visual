"""Python consumers export the exact retained multiset and reject stale previews."""

import http.client
import json
import xml.etree.ElementTree as ET
from collections import Counter

import pytest

import kglite_visual as kv
from conftest import REPO_ROOT, get_json
from test_saved_queries import post_json


def raw_request(port, method, route, data=None):
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=15)
    try:
        connection.request(method, route, None if data is None else json.dumps(data),
                           {} if data is None else {'Content-Type': 'application/json'})
        response = connection.getresponse()
        return response.status, dict(response.getheaders()), response.read()
    finally:
        connection.close()


def gexf_members(payload):
    document = ET.fromstring(payload)
    nodes = document.findall('.//{*}node')
    edges = document.findall('.//{*}edge')
    assert len({node.attrib['id'] for node in nodes}) == len(nodes)
    assert len({edge.attrib['id'] for edge in edges}) == len(edges)
    return len(nodes), Counter((edge.attrib['source'], edge.attrib['target']) for edge in edges)


@pytest.mark.parametrize('as_bytes', [False, True])
def test_scoped_export_preserves_parallel_and_self_edges_and_refuses_stale_preview(as_bytes, tmp_path, monkeypatch):
    monkeypatch.setenv('KGLITE_VISUAL_CONFIG_DIR', str(tmp_path))
    path = REPO_ROOT / 'crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl'
    with kv.show(path.read_bytes() if as_bytes else str(path), open_browser=False) as view:
        status, nodes = post_json(view.port, '/api/cypher', {
            'query': 'MATCH (n:Person) RETURN n', 'as_graph': False,
        })
        assert status == 200, nodes
        handles = [row['nodes'][0] for row in nodes['row_references']]
        status, table = post_json(view.port, '/api/cypher', {
            'query': 'MATCH (a)-[r]->(b) RETURN r', 'as_graph': False,
        })
        assert status == 200, table
        relationships = sorted([row['relationships'][0] for row in table['row_references']],
                               key=lambda edge: edge['edge_id'])
        assert [edge['edge_id'] for edge in relationships] == [0, 1, 2, 3]
        status, loaded = post_json(view.port, '/api/load-entities', {
            'nodes': handles, 'relationships': relationships[:3], 'expected': table['stamp'],
        })
        assert status == 200, loaded
        before = get_json(view.port, '/api/view-state')
        assert before['subset']['counts']['visible_nodes'] == 3
        assert before['subset']['visible_edge_ids'] == [0, 1, 2]
        request = {'scope': 'visible', 'format': 'gexf', 'expected': before['stamp'],
                   'subset_revision': before['subset_revision']}
        status, preview = post_json(view.port, '/api/export/preview', request)
        assert status == 200, preview
        download = {**request, 'preview_digest': preview['preview_digest']}
        status, headers, payload = raw_request(view.port, 'POST', '/api/export/download', download)
        assert status == 200, payload
        count, edges = gexf_members(payload)
        assert count == 3
        assert sum(edges.values()) == 3
        assert sum(value for (source, target), value in edges.items() if source == target) == 1
        assert sorted(edges.values()) == [1, 2]
        assert headers['x-kglv-scope'] == 'visible'
        status, _, legacy = raw_request(view.port, 'GET', '/api/export?format=gexf&source=live-view')
        assert status == 200, legacy
        assert gexf_members(legacy)[0] == 3
        assert sum(gexf_members(legacy)[1].values()) == 4
        assert get_json(view.port, '/api/view-state') == before

        status, caption = post_json(view.port, '/api/caption', {'caption_by': 'id', 'expected': before['stamp']})
        assert status == 200, caption
        changed = get_json(view.port, '/api/view-state')
        status, _, refused = raw_request(view.port, 'POST', '/api/export/download', download)
        assert status == 409, refused
        assert get_json(view.port, '/api/view-state') == changed
