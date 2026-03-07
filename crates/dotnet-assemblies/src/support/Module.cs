using JetBrains.Annotations;

namespace DotnetRs;

public class Module : System.Reflection.Module
{
    [UsedImplicitly] private IntPtr resolution;
}