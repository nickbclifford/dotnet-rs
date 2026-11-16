using System.Reflection;

namespace DotnetRs;

// The loader currently represents one manifest module per assembly. Cache the corresponding
// reflection object by assembly so all types in that module report the same object.
public class Module : System.Reflection.Module
{
    private static readonly Dictionary<System.Reflection.Assembly, Module> Modules = new();
    internal static readonly Module Anonymous = new(new AnonymousAssembly());
    private readonly System.Reflection.Assembly _assembly;

    private Module(System.Reflection.Assembly assembly) => _assembly = assembly;

    internal static Module ForAssembly(System.Reflection.Assembly assembly)
    {
        lock (Modules)
        {
            if (!Modules.TryGetValue(assembly, out var module))
                Modules.Add(assembly, module = new Module(assembly));
            return module;
        }
    }

    public override System.Reflection.Assembly Assembly => _assembly;
    public override IList<CustomAttributeData> GetCustomAttributesData() => new List<CustomAttributeData>();

    private sealed class AnonymousAssembly : System.Reflection.Assembly
    {
        public override string? FullName => "Anonymously Hosted DynamicMethods Assembly, Version=0.0.0.0, Culture=neutral, PublicKeyToken=null";
        public override AssemblyName GetName(bool copiedName) => new("Anonymously Hosted DynamicMethods Assembly");
    }
}
