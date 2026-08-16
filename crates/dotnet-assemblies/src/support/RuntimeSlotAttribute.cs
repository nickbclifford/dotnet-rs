namespace DotnetRs;

[AttributeUsage(AttributeTargets.Field)]
internal sealed class RuntimeSlotAttribute : Attribute
{
    // Valid values are Handle, Index, GcRef, Byref, ScalarInt, and NativePtr.
    // They are validated by the Rust support-contract loader.
    public RuntimeSlotAttribute(string kind)
    {
        Kind = kind;
    }

    public string Kind { get; }
}
