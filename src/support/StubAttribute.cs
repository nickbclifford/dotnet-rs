namespace DotnetRs;

[AttributeUsage(AttributeTargets.Class |
                       AttributeTargets.Struct)
]
public class StubAttribute : Attribute
{
    public required string InPlaceOf;
}