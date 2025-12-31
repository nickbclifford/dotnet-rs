using JetBrains.Annotations;

namespace DotnetRs;

[AttributeUsage(AttributeTargets.Class |
                       AttributeTargets.Struct)
]
public class StubAttribute : Attribute
{
    [UsedImplicitly] public required string InPlaceOf;
}