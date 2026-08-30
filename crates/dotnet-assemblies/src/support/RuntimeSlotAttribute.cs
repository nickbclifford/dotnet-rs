namespace DotnetRs;

[AttributeUsage(AttributeTargets.Field)]
internal sealed class RuntimeSlotAttribute : Attribute
{
    public RuntimeSlotAttribute(RuntimeSlotId id)
    {
        Id = id;
    }

    public RuntimeSlotId Id { get; }
}
