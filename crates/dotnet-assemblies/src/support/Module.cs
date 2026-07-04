using JetBrains.Annotations;

namespace DotnetRs;

public class Module : System.Reflection.Module
{
    [UsedImplicitly] private IntPtr resolution;

    public override System.Collections.Generic.IList<System.Reflection.CustomAttributeData> GetCustomAttributesData() =>
        new System.Collections.Generic.List<System.Reflection.CustomAttributeData>();
}