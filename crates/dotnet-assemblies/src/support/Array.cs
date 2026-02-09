using System.Collections;
using System.Runtime.CompilerServices;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Array")]
public class Array : ICloneable, IList, IStructuralComparable, IStructuralEquatable
{
    // Sentinel for array constructors used by runtime
    internal void CtorArraySentinel() { }

    public extern int Length { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public long LongLength => GetLongLength(0);

    public nuint NativeLength => (nuint)Length;

    public extern int Rank { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public virtual int GetLength(int dimension) => throw new NotImplementedException();

    public long GetLongLength(int dimension) => GetLength(dimension);

    public virtual int GetRank() => throw new NotImplementedException();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern object? GetValue(int index);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern object? GetValue(long index);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public virtual extern object? GetValue(params int[] indices);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern void SetValue(object? value, int index);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern void SetValue(object? value, long index);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public virtual extern void SetValue(object? value, params int[] indices);

    public object Clone() => MemberwiseClone();

    // IList implementation
    public bool IsFixedSize => true;
    public bool IsReadOnly => false;
    public bool IsSynchronized => false;
    public object SyncRoot => this;
    public int Count => Length;

    object? IList.this[int index]
    {
        get => GetValue(index);
        set => SetValue(value, index);
    }

    int IList.Add(object? value) => throw new NotSupportedException();
    void IList.Clear() => throw new NotSupportedException();
    bool IList.Contains(object? value) => IndexOf(this, value) >= 0;
    int IList.IndexOf(object? value) => IndexOf(this, value);
    void IList.Insert(int index, object? value) => throw new NotSupportedException();
    void IList.Remove(object? value) => throw new NotSupportedException();
    void IList.RemoveAt(int index) => throw new NotSupportedException();

    public IEnumerator GetEnumerator() => new ArrayEnumerator(this);

    public void CopyTo(System.Array array, int index)
    {
        ArgumentNullException.ThrowIfNull(array);
        if (Rank != 1) throw new RankException();
        ArgumentOutOfRangeException.ThrowIfNegative(index);
        if (index + Length > array.Length) throw new ArgumentException();

        for (int i = 0; i < Length; i++)
        {
            array.SetValue(GetValue(i), index + i);
        }
    }

    // IStructuralComparable / IStructuralEquatable stubs
    int IStructuralComparable.CompareTo(object? other, IComparer comparer) => throw new NotImplementedException();
    bool IStructuralEquatable.Equals(object? other, IEqualityComparer comparer) => throw new NotImplementedException();
    int IStructuralEquatable.GetHashCode(IEqualityComparer comparer) => throw new NotImplementedException();

    // Static helper methods
    public static int IndexOf(Array array, object? value)
    {
        ArgumentNullException.ThrowIfNull(array);
        return IndexOf(array, value, 0, array.Length);
    }

    public static int IndexOf(Array array, object? value, int startIndex)
    {
        ArgumentNullException.ThrowIfNull(array);
        return IndexOf(array, value, startIndex, array.Length - startIndex);
    }

    public static int IndexOf(Array array, object? value, int startIndex, int count)
    {
        ArgumentNullException.ThrowIfNull(array);
        if (startIndex < 0 || startIndex > array.Length) throw new ArgumentOutOfRangeException(nameof(startIndex));
        if (count < 0 || startIndex + count > array.Length) throw new ArgumentOutOfRangeException(nameof(count));

        for (int i = startIndex; i < startIndex + count; i++)
        {
            var item = array.GetValue(i);
            if (item == null && value == null) return i;
            if (item != null && item.Equals(value)) return i;
        }
        return -1;
    }

    public static int LastIndexOf(Array array, object? value)
    {
        ArgumentNullException.ThrowIfNull(array);
        return LastIndexOf(array, value, array.Length - 1, array.Length);
    }

    public static int LastIndexOf(Array array, object? value, int startIndex)
    {
        ArgumentNullException.ThrowIfNull(array);
        return LastIndexOf(array, value, startIndex, startIndex + 1);
    }

    public static int LastIndexOf(Array array, object? value, int startIndex, int count)
    {
        ArgumentNullException.ThrowIfNull(array);
        if (array.Length == 0) return -1;
        if (startIndex < 0 || startIndex >= array.Length) throw new ArgumentOutOfRangeException(nameof(startIndex));
        if (count < 0 || startIndex - count + 1 < 0) throw new ArgumentOutOfRangeException(nameof(count));

        for (int i = startIndex; i > startIndex - count; i--)
        {
            var item = array.GetValue(i);
            if (item == null && value == null) return i;
            if (item != null && item.Equals(value)) return i;
        }
        return -1;
    }

    public static int IndexOf<T>(T[] array, T value)
    {
        return IndexOf((Array)(object)array, value, 0, array.Length);
    }

    public static int IndexOf<T>(T[] array, T value, int startIndex)
    {
        return IndexOf((Array)(object)array, value, startIndex, array.Length - startIndex);
    }

    public static int IndexOf<T>(T[] array, T value, int startIndex, int count)
    {
        return IndexOf((Array)(object)array, value, startIndex, count);
    }

    public static int LastIndexOf<T>(T[] array, T value)
    {
        ArgumentNullException.ThrowIfNull(array);
        return LastIndexOf((Array)(object)array, value, array.Length - 1, array.Length);
    }

    public static int LastIndexOf<T>(T[] array, T value, int startIndex)
    {
        ArgumentNullException.ThrowIfNull(array);
        return LastIndexOf((Array)(object)array, value, startIndex, startIndex + 1);
    }

    public static int LastIndexOf<T>(T[] array, T value, int startIndex, int count)
    {
        return LastIndexOf((Array)(object)array, value, startIndex, count);
    }

    public static void Copy(Array sourceArray, Array destinationArray, int length)
    {
        Copy(sourceArray, 0, destinationArray, 0, length);
    }

    public static void Copy(Array sourceArray, int sourceIndex, Array destinationArray, int destinationIndex, int length)
    {
        ArgumentNullException.ThrowIfNull(sourceArray);
        ArgumentNullException.ThrowIfNull(destinationArray);
        ArgumentOutOfRangeException.ThrowIfNegative(length);
        ArgumentOutOfRangeException.ThrowIfNegative(sourceIndex);
        ArgumentOutOfRangeException.ThrowIfNegative(destinationIndex);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(sourceIndex + length, sourceArray.Length);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(destinationIndex + length, destinationArray.Length);

        for (int i = 0; i < length; i++)
        {
            destinationArray.SetValue(sourceArray.GetValue(sourceIndex + i), destinationIndex + i);
        }
    }

    public static void Clear(Array array)
    {
        ArgumentNullException.ThrowIfNull(array);
        Clear(array, 0, array.Length);
    }

    public static void Clear(Array array, int index, int length)
    {
        ArgumentNullException.ThrowIfNull(array);
        ArgumentOutOfRangeException.ThrowIfNegative(index);
        ArgumentOutOfRangeException.ThrowIfNegative(length);
        if (index + length > array.Length) throw new ArgumentException();

        for (int i = index; i < index + length; i++)
        {
            array.SetValue(null, i);
        }
    }

    public static T[] Empty<T>() => new T[0];

    private class ArrayEnumerator(Array array) : IEnumerator
    {
        private int _index = -1;

        public object? Current => array.GetValue(_index);

        public bool MoveNext()
        {
            if (_index < array.Length - 1)
            {
                _index++;
                return true;
            }
            return false;
        }

        public void Reset() => _index = -1;
    }
}
