"""Independent SQL oracle for the new migration baseline, never an old acceptance date."""
from contextlib import closing
import sqlite3


def recorded_history(path):
    with closing(sqlite3.connect(path)) as connection:
        table = connection.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='head_observations'"
        ).fetchone()
        if table is None:
            return None
        return table[0], connection.execute(
            'SELECT * FROM head_observations ORDER BY rowid'
        ).fetchall()


def assert_recorded_baselines(path, migration_time):
    with closing(sqlite3.connect(path)) as connection:
        expected = sorted((graph, branch, revision, migration_time, 'baseline', None)
                          for graph, branch, revision in connection.execute(
                              'SELECT graph_id,branch_id,revision FROM heads'))
        actual = sorted(connection.execute(
            'SELECT graph_id,branch_id,revision,recorded_at_ms,kind,parent FROM head_observations'
        ).fetchall())
        assert actual == expected, (actual, expected)
    return recorded_history(path)
