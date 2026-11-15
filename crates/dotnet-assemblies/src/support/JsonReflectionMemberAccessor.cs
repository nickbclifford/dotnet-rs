namespace DotnetRs;

/// <summary>
/// Supplies System.Text.Json's <c>ReflectionMemberAccessor</c> to
/// <c>DefaultJsonTypeInfoResolver.MemberAccessor</c>. The VM routes that getter here because the
/// library chooses its Reflection.Emit accessor whenever <c>IsDynamicCodeSupported</c> is true.
/// </summary>
public static class JsonReflectionMemberAccessor
{
    private static object? s_instance;

    public static object Get(Type accessorType)
    {
        object? instance = s_instance;
        if (instance == null)
        {
            instance = Activator.CreateInstance(accessorType, nonPublic: true)!;
            instance = System.Threading.Interlocked.CompareExchange(ref s_instance, instance, null) ?? instance;
        }

        return instance;
    }
}
