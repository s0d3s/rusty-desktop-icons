"""String-compatible choices accepted by the native bindings."""

from enum import Enum


class _StringEnum(str, Enum):
    def __str__(self) -> str:
        return self.value


class KeyframeInterp(_StringEnum):
    """Interpolation between sampled or explicit keyframes."""

    Linear = "linear"
    Step = "step"
    SmoothStep = "smoothstep"


class ShaderPipeline(_StringEnum):
    """Geometry pipeline used by a shader source."""

    Sprite = "sprite"
    Particles = "particles"
    Procedural = "procedural"


class FolderFlagOpKind(_StringEnum):
    """Folder-flag operation discriminator for dict and tuple inputs."""

    Set = "set"
    Exactly = "exactly"


class CurveKind(_StringEnum):
    """Representation of an animation curve."""

    Builtin = "builtin"
    Keyframe = "keyframe"


class DurationKind(_StringEnum):
    """How an animation duration is resolved."""

    Fixed = "fixed"
    Distance = "distance"


class FinishReasonKind(_StringEnum):
    """Discriminator of an animation's finish reason."""

    Completed = "completed"
    Stopped = "stopped"
    Error = "error"


class LogTarget(_StringEnum):
    """Destination for native tracing events."""

    Loguru = "loguru"
    Stderr = "stderr"
