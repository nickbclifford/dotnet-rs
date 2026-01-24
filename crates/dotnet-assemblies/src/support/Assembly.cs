using JetBrains.Annotations;

namespace DotnetRs;

public class Assembly : System.Reflection.Assembly
{
    [UsedImplicitly] private nint resolution;
}