import { describe, expect, it } from 'vitest';
import {
  BODY_OPTIONS,
  bodyLabel,
  bodyValue,
  destinationActionText,
  destinationIcon,
  destinationStatusText,
  errorText,
  METHOD_OPTIONS,
  pathDisplay,
  pathValue,
} from './model';

describe('destination form mappings', () => {
  it('maps the body labels to ShareX values and back', () => {
    expect(METHOD_OPTIONS).toEqual(['POST', 'PUT', 'PATCH']);
    expect(BODY_OPTIONS).toEqual(['Multipart form data', 'Binary', 'Form URL-encoded', 'JSON']);
    expect(bodyValue('Multipart form data')).toBe('MultipartFormData');
    expect(bodyValue('Binary')).toBe('Binary');
    expect(bodyValue('Form URL-encoded')).toBe('FormURLEncoded');
    expect(bodyValue('JSON')).toBe('JSON');
    expect(bodyLabel('MultipartFormData')).toBe('Multipart form data');
    expect(bodyLabel(null)).toBe('Multipart form data');
  });

  it('shows JSON paths but stores ShareX templates', () => {
    expect(pathDisplay('{json:$.data.url}')).toBe('$.data.url');
    expect(pathDisplay('{json:data.url}')).toBe('$.data.url');
    expect(pathDisplay('{json:[0].url}')).toBe('$[0].url');
    expect(pathValue('$.data.url')).toBe('{json:$.data.url}');
    expect(pathValue('{response}')).toBe('{response}');
    expect(pathValue('')).toBe('');
    expect(pathDisplay(null)).toBe('');
  });

  it('uses the exact status and action labels', () => {
    expect(destinationStatusText(true)).toBe('Configured');
    expect(destinationStatusText(false)).toBe('Not set up');
    expect(destinationActionText('configure')).toBe('Configure');
    expect(destinationActionText('set_up')).toBe('Set up');
    expect(destinationActionText('edit')).toBe('Edit');
  });

  it('gives each destination kind its icon', () => {
    expect(destinationIcon('imgur')).toBe('image');
    expect(destinationIcon('s3')).toBe('bucket');
    expect(destinationIcon('gist')).toBe('code');
    expect(destinationIcon('sftp')).toBe('server');
    expect(destinationIcon('custom')).toBe('cloud');
  });

  it('reads an IPC message without exposing anything else', () => {
    expect(errorText({ message: 'The upload destination was not found.' })).toBe(
      'The upload destination was not found.',
    );
    expect(errorText(null)).toBe('The destination could not be saved.');
  });
});
