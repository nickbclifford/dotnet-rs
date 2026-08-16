using JetBrains.Annotations;

namespace DotnetRs;

[AttributeUsage(AttributeTargets.Class |
                       AttributeTargets.Struct)
]
public class StubAttribute : Attribute
{
    [RuntimeSlot("GcRef")]
    [UsedImplicitly] public required string InPlaceOf;
}
