namespace DotnetRs;

public class Module : System.Reflection.Module
{
    public override IList<System.Reflection.CustomAttributeData> GetCustomAttributesData() =>
        new List<System.Reflection.CustomAttributeData>();
}