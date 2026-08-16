using JetBrains.Annotations;

namespace DotnetRs;

public class Module : System.Reflection.Module
{
    [RuntimeSlot("NativePtr")]
    [UsedImplicitly] private IntPtr resolution;

    public override IList<System.Reflection.CustomAttributeData> GetCustomAttributesData() =>
        new List<System.Reflection.CustomAttributeData>();
}