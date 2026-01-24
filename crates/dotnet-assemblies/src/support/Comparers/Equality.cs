using System.Collections;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs.Comparers;

public class Equality
{
    [UsedImplicitly]
    internal static IEqualityComparer GetDefault(RuntimeType type)
    {
        // https://github.com/dotnet/runtime/blob/main/src/coreclr/System.Private.CoreLib/src/System/Collections/Generic/ComparerHelpers.cs#L62

        if (type == typeof(string))
        {
            return StringComparer.InvariantCulture;
        }

        Type comparerType;

        if (type.IsAssignableTo(typeof(IEquatable<>).MakeGenericType(type)))
        {
            comparerType = typeof(GenericEqualityComparer<>).MakeGenericType(type);
        }
        else if (type.IsGenericType && type.GetGenericTypeDefinition() == typeof(Nullable<>))
        {
            comparerType = typeof(NullableEqualityComparer<>).MakeGenericType(type.GenericTypeArguments[0]);
        }
        else if (type.IsEnum)
        {
            comparerType = typeof(EnumEqualityComparer<>).MakeGenericType(type);
        }
        else
        {
            comparerType = typeof(ObjectEqualityComparer<>).MakeGenericType(type);
        }
        
        return (IEqualityComparer) Activator.CreateInstance(comparerType)!;
    }
    
    // implementations taken from System.Private.CoreLib since theirs are inaccessible
    // https://github.com/dotnet/runtime/blob/main/src/libraries/System.Private.CoreLib/src/System/Collections/Generic/EqualityComparer.cs
    
    public class GenericEqualityComparer<T> : EqualityComparer<T> where T : IEquatable<T>?
    {
        public override bool Equals(T? x, T? y)
        {
            if (x != null)
            {
                return y != null && x.Equals(y);
            }
            return y == null;
        }
        
        public override int GetHashCode(T obj) => obj?.GetHashCode() ?? 0;
        public override bool Equals(object? obj) => obj != null && GetType() == obj.GetType();
        public override int GetHashCode() => GetType().GetHashCode();
    }

    public class NullableEqualityComparer<T> : EqualityComparer<T?> where T : struct
    {
        public override bool Equals(T? x, T? y)
        {
            if (x.HasValue)
            {
                return y.HasValue && EqualityComparer<T>.Default.Equals(x.Value, y.Value);
            }
            return !y.HasValue;
        }

        public override int GetHashCode(T? obj) => obj.GetHashCode();
        public override bool Equals(object? obj) => obj != null && GetType() == obj.GetType();
        public override int GetHashCode() => GetType().GetHashCode();
    }

    public class EnumEqualityComparer<T> : EqualityComparer<T> where T : struct, Enum
    {
        [MethodImpl(MethodImplOptions.InternalCall)]
        public override extern bool Equals(T x, T y);

        public override int GetHashCode(T obj) => obj.GetHashCode();
        public override bool Equals(object? obj) => obj != null && GetType() == obj.GetType();
        public override int GetHashCode() => GetType().GetHashCode();
    }

    public class ObjectEqualityComparer<T> : EqualityComparer<T>
    {
        public override bool Equals(T? x, T? y)
        {
            if (x != null)
            {
                return y != null && x.Equals(y);
            }
            return y == null;
        }

        public override int GetHashCode(T obj) => obj?.GetHashCode() ?? 0;
        public override bool Equals(object? obj) => obj != null && GetType() == obj.GetType();
        public override int GetHashCode() => GetType().GetHashCode();
    }
}