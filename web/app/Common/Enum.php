<?php

namespace Wikijump\Common;

use http\Exception\RuntimeException;

/**
 * Implements an enumeration abstract class for the creation of enums within PHP.
 *
 * Children should make a final class that extends Enum, and then add all the
 * variants as constants for enum values.
 *
 * The static methods values() and isValue() are available for use.
 *
 * See https://stackoverflow.com/questions/254514/enumerations-on-php/17045081#17045081
 *
 * @package Wikijump\Common
 */
abstract class Enum
{
    final private function __construct()
    {
        throw new RuntimeException("You may not create Enum class instances");
    }

    final private function __clone()
    {
        throw new RuntimeException("You may not clone Enum class instances");
    }

    final public static function values(): array
    {
        $class = new ReflectionClass(static::class);
        return $class->getConstants();
    }

    final public static function isValue(mixed $value): bool
    {
        return in_array($value, static::values());
    }
}
