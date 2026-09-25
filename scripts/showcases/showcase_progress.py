"""Parent-owned progress reporting for showcase command-line tools."""

from __future__ import annotations

from collections.abc import Callable, Iterator
from contextlib import ExitStack, contextmanager
from contextvars import ContextVar
from dataclasses import dataclass
from functools import wraps
from inspect import signature
import multiprocessing
from queue import Queue
from threading import Thread
from time import monotonic
from typing import ParamSpec, Protocol, TypeVar
from uuid import uuid4

from rich.console import Console
from rich.progress import (BarColumn, MofNCompleteColumn, Progress, SpinnerColumn,
                           TaskID, TaskProgressColumn, TextColumn, TimeElapsedColumn, TimeRemainingColumn)
from rich.text import Text


@dataclass
class Event:
    kind: str
    key: str = ""
    label: str = ""
    stage: str = ""
    completed: float = 0
    total: float | None = None


class EventQueue(Protocol):
    def put(self, event: Event | None) -> None: ...
    def get(self) -> Event | None: ...


@dataclass
class Work:
    key: str
    stage: str = ""
    total: float | None = None
    last_update: float = 0
    outcome: str = "Done"


_events: EventQueue | None = None
_work: ContextVar[Work | None] = ContextVar("showcase_work", default=None)
Parameters = ParamSpec("Parameters")
Result = TypeVar("Result")


def connect(events: EventQueue | None) -> None:
    global _events
    _events = events


def connection() -> EventQueue | None:
    return _events


def message(text: str) -> None:
    if _events is None:
        print(text, flush=True)
    else:
        _events.put(Event("message", label=text))


def report(stage: str, completed: float = 0, total: float | None = None) -> None:
    work = _work.get()
    if work is None or _events is None:
        return
    now = monotonic()
    changed = stage != work.stage or total != work.total
    work.stage, work.total = stage, total
    if changed or (total is not None and completed >= total) or now - work.last_update >= 0.1:
        _events.put(Event("update", work.key, stage=stage, completed=completed, total=total))
        work.last_update = now


def advance_to(completed: float) -> None:
    work = _work.get()
    if work is not None:
        report(work.stage, min(completed, work.total) if work.total is not None else completed, work.total)


def skipped(outcome: str = "Skipped") -> None:
    work = _work.get()
    if work is not None:
        work.outcome = outcome


def work_item(argument: str | None = None) -> Callable[[Callable[Parameters, Result]], Callable[Parameters, Result]]:
    def decorate(function: Callable[Parameters, Result]) -> Callable[Parameters, Result]:
        parameters = signature(function)

        @wraps(function)
        def wrapped(*args: Parameters.args, **kwargs: Parameters.kwargs) -> Result:
            if _events is None or _work.get() is not None:
                return function(*args, **kwargs)
            value = parameters.bind(*args, **kwargs).arguments[argument] if argument else function.__name__.replace("_", " ")
            label = str(getattr(value, "name", value))
            work = Work(uuid4().hex)
            token = _work.set(work)
            _events.put(Event("start", work.key, label=label, stage="Starting"))
            try:
                return function(*args, **kwargs)
            except BaseException as error:
                work.outcome = "Cancelled" if isinstance(error, KeyboardInterrupt) else "Failed"
                message(f"{work.outcome} {label}: {error}")
                raise
            finally:
                _events.put(Event("end", work.key, stage=work.outcome))
                _work.reset(token)
        return wrapped
    return decorate


@contextmanager
def display(title: str, total: int, *, enabled: bool = True, processes: bool = False,
            console: Console | None = None) -> Iterator[EventQueue]:
    console = console or Console()
    with ExitStack() as stack:
        events: EventQueue = (stack.enter_context(multiprocessing.get_context("spawn").Manager()).Queue()
                              if processes else Queue())
        previous = _events
        progress = stack.enter_context(Progress(
            SpinnerColumn("line"), TextColumn("{task.description}", markup=False), BarColumn(), TaskProgressColumn(),
            MofNCompleteColumn(), TextColumn("{task.fields[stage]}", markup=False),
            TimeElapsedColumn(), TimeRemainingColumn(), console=console,
            disable=not enabled or not console.is_terminal, refresh_per_second=8,
        ))
        connect(events)
        overall = progress.add_task(title, total=total, stage="items")
        tasks: dict[str, TaskID] = {}

        def consume() -> None:
            while (event := events.get()) is not None:
                if event.kind == "message":
                    console.print(Text(event.label), soft_wrap=True)
                elif event.kind == "start":
                    tasks[event.key] = progress.add_task(event.label, total=None, stage=event.stage)
                elif event.kind == "update":
                    task = tasks[event.key]
                    if progress.tasks[task].fields["stage"] != event.stage:
                        progress.reset(task, total=event.total, completed=event.completed, stage=event.stage)
                    else:
                        progress.update(task, total=event.total, completed=event.completed)
                elif event.kind == "end":
                    task = tasks[event.key]
                    if event.stage in ("Done", "Skipped", "Exists"):
                        progress.update(task, total=1, completed=1)
                    progress.update(task, stage=event.stage)
                    progress.stop_task(task)
                    progress.advance(overall)

        listener = Thread(target=consume, name="showcase-progress", daemon=True)
        listener.start()
        try:
            yield events
        finally:
            events.put(None)
            listener.join()
            for task in progress.tasks[1:]:
                if task.fields["stage"] not in ("Done", "Skipped", "Exists", "Failed", "Cancelled"):
                    progress.update(task.id, stage="Cancelled")
                    progress.stop_task(task.id)
            connect(previous)