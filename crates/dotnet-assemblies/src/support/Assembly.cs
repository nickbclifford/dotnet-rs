using JetBrains.Annotations;

namespace DotnetRs;

public class Assembly : System.Reflection.Assembly
{
    [RuntimeSlot("NativePtr")]
    [UsedImplicitly] private nint resolution;
}